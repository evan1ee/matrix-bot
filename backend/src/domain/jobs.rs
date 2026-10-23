//! The durable job queue's vocabulary: which kinds of work exist, the retry
//! policy and dedupe key each kind uses, and the pure rules the runtime
//! applies (backoff, error truncation). Persistence is in
//! `infrastructure/postgres/repositories/jobs.rs`; execution in `crate::jobs`.

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;
use crate::domain::channels::ConversationAddress;
use crate::domain::finance::budget::{Month, Threshold};
use crate::domain::knowledge::search::KnowledgeItemType;
use crate::domain::reminders::MAX_DELIVERY_ATTEMPTS;
use crate::domain::storage::StorageKey;

/// Longest error text stored on a job row.
pub const MAX_ERROR_CHARS: usize = 2_000;
/// Upper bound on any computed backoff.
pub const MAX_BACKOFF: Duration = Duration::from_secs(86_400);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub i64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobType {
    ReminderFire,
    BriefingSend,
    PortfolioCheckAlerts,
    PortfolioRefreshPrices,
    MediaProcessReceipt,
    AgentExecuteRun,
    KnowledgeEmbedItem,
    KnowledgeBackfillEmbeddings,
    BudgetAlertSend,
}

impl JobType {
    pub const ALL: [JobType; 9] = [
        JobType::ReminderFire,
        JobType::BriefingSend,
        JobType::PortfolioCheckAlerts,
        JobType::PortfolioRefreshPrices,
        JobType::MediaProcessReceipt,
        JobType::AgentExecuteRun,
        JobType::KnowledgeEmbedItem,
        JobType::KnowledgeBackfillEmbeddings,
        JobType::BudgetAlertSend,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            JobType::ReminderFire => "reminder.fire",
            JobType::BriefingSend => "briefing.send",
            JobType::PortfolioCheckAlerts => "portfolio.check_alerts",
            JobType::PortfolioRefreshPrices => "portfolio.refresh_prices",
            JobType::MediaProcessReceipt => "media.process_receipt",
            JobType::AgentExecuteRun => "agent.execute_run",
            JobType::KnowledgeEmbedItem => "knowledge.embed_item",
            JobType::KnowledgeBackfillEmbeddings => "knowledge.backfill_embeddings",
            JobType::BudgetAlertSend => "finance.budget_alert",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Pending,
            Self::Running,
            Self::Completed,
            Self::Failed,
            Self::Cancelled,
        ]
        .into_iter()
        .find(|st| st.as_str() == s)
    }
}

/// `reminder.fire`: a row id plus the trigger the job was enqueued for, so a
/// handler can tell a snoozed/advanced reminder from the one it was meant for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderFirePayload {
    pub reminder_id: i64,
    pub trigger_at: DateTime<Utc>,
}

/// `briefing.send`: the recipient's local date; the account is the job's
/// `account_id` column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefingSendPayload {
    pub local_date: NaiveDate,
}

/// `media.process_receipt`: an uploaded receipt image in object storage to scan
/// and parse into a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessReceiptPayload {
    pub storage_key: StorageKey,
    pub account_id: AccountId,
    pub conversation: Option<ConversationAddress>,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExecutePayload {
    pub run_id: uuid::Uuid,
    pub origin_conversation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeEmbedPayload {
    pub account_id: AccountId,
    pub item_type: KnowledgeItemType,
    pub item_id: i64,
}

/// `finance.budget_alert`: the budget, month and threshold (80 or 100) that was
/// crossed; the account is the job's `account_id` column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetAlertPayload {
    pub budget_id: i64,
    pub month: Month,
    pub threshold: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobPayload {
    BudgetAlert(BudgetAlertPayload),
    ReminderFire(ReminderFirePayload),
    BriefingSend(BriefingSendPayload),
    MediaProcessReceipt(ProcessReceiptPayload),
    AgentExecute(AgentExecutePayload),
    KnowledgeEmbed(KnowledgeEmbedPayload),
    /// `{}` — the job's type and account say everything.
    Empty,
}

/// A job ready to enqueue. Built only through the per-type constructors, so
/// a job's type, payload, retry policy and dedupe key always agree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    job_type: JobType,
    account_id: Option<AccountId>,
    payload: JobPayload,
    priority: i16,
    max_attempts: i32,
    backoff_seconds: i32,
    available_at: Option<DateTime<Utc>>,
    dedupe_key: Option<String>,
}

impl NewJob {
    fn new(
        job_type: JobType,
        account_id: Option<AccountId>,
        payload: JobPayload,
        max_attempts: i32,
        backoff_seconds: i32,
        dedupe_key: String,
    ) -> Self {
        Self {
            job_type,
            account_id,
            payload,
            priority: 0,
            max_attempts,
            backoff_seconds,
            available_at: None,
            dedupe_key: Some(dedupe_key),
        }
    }

    pub fn reminder_fire(owner: AccountId, reminder_id: i64, trigger_at: DateTime<Utc>) -> Self {
        let key = format!(
            "{reminder_id}:{}",
            trigger_at.to_rfc3339_opts(SecondsFormat::Micros, true)
        );
        let payload = JobPayload::ReminderFire(ReminderFirePayload {
            reminder_id,
            trigger_at,
        });
        // MAX_DELIVERY_ATTEMPTS is 5; the cast cannot truncate.
        Self::new(
            JobType::ReminderFire,
            Some(owner),
            payload,
            MAX_DELIVERY_ATTEMPTS as i32,
            30,
            key,
        )
    }

    pub fn briefing_send(owner: AccountId, local_date: NaiveDate) -> Self {
        let key = format!("{owner}:{}", local_date.format("%Y-%m-%d"));
        let payload = JobPayload::BriefingSend(BriefingSendPayload { local_date });
        Self::new(JobType::BriefingSend, Some(owner), payload, 3, 60, key)
    }

    pub fn portfolio_check_alerts(now: DateTime<Utc>) -> Self {
        let key = format!("alerts:{}", now.timestamp().div_euclid(60));
        Self::new(
            JobType::PortfolioCheckAlerts,
            None,
            JobPayload::Empty,
            1,
            0,
            key,
        )
    }

    pub fn portfolio_refresh_prices(owner: AccountId, now: DateTime<Utc>) -> Self {
        let key = format!("{owner}:{}", now.format("%Y-%m-%dT%H"));
        Self::new(
            JobType::PortfolioRefreshPrices,
            Some(owner),
            JobPayload::Empty,
            3,
            60,
            key,
        )
    }

    pub fn media_process_receipt(
        owner: AccountId,
        storage_key: StorageKey,
        conversation: Option<ConversationAddress>,
        caption: Option<String>,
    ) -> Self {
        let dedupe_key = storage_key.as_str().to_owned();
        let payload = JobPayload::MediaProcessReceipt(ProcessReceiptPayload {
            storage_key,
            account_id: owner,
            conversation,
            caption,
        });
        Self::new(
            JobType::MediaProcessReceipt,
            Some(owner),
            payload,
            3,
            60,
            dedupe_key,
        )
    }

    pub fn agent_execute(
        owner: AccountId,
        run_id: uuid::Uuid,
        origin_conversation: Option<String>,
    ) -> Self {
        let dedupe_key = format!("agent:{run_id}:{}", uuid::Uuid::new_v4());
        let payload = JobPayload::AgentExecute(AgentExecutePayload {
            run_id,
            origin_conversation,
        });
        Self::new(
            JobType::AgentExecuteRun,
            Some(owner),
            payload,
            3,
            30,
            dedupe_key,
        )
    }

    /// Embeds an item once. The key is permanent until finished jobs are
    /// purged, so use [`Self::knowledge_embed_item_versioned`] for anything that
    /// can change and need embedding again.
    pub fn knowledge_embed_item(
        owner: AccountId,
        item_type: KnowledgeItemType,
        item_id: i64,
    ) -> Self {
        Self::knowledge_embed(
            owner,
            item_type,
            item_id,
            format!("{}:{item_id}", item_type.as_str()),
        )
    }

    /// Embeds one version of an item. A new `version` (an edit, a model switch)
    /// gets a new key, so it is not swallowed by the finished job of the
    /// version before it, while a repeated request for the same version is
    /// still deduplicated.
    pub fn knowledge_embed_item_versioned(
        owner: AccountId,
        item_type: KnowledgeItemType,
        item_id: i64,
        version: &str,
    ) -> Self {
        Self::knowledge_embed(
            owner,
            item_type,
            item_id,
            format!("{}:{item_id}:{version}", item_type.as_str()),
        )
    }

    fn knowledge_embed(
        owner: AccountId,
        item_type: KnowledgeItemType,
        item_id: i64,
        dedupe_key: String,
    ) -> Self {
        let payload = JobPayload::KnowledgeEmbed(KnowledgeEmbedPayload {
            account_id: owner,
            item_type,
            item_id,
        });
        Self::new(
            JobType::KnowledgeEmbedItem,
            Some(owner),
            payload,
            3,
            30,
            dedupe_key,
        )
    }

    /// One backfill per UTC hour: the scheduler ticks hourly, and a key that
    /// never changed would let the first finished run block every later one.
    pub fn knowledge_backfill_embeddings(now: DateTime<Utc>) -> Self {
        let dedupe_key = format!("knowledge:backfill:{}", now.format("%Y-%m-%dT%H"));
        Self::new(
            JobType::KnowledgeBackfillEmbeddings,
            None,
            JobPayload::Empty,
            1,
            0,
            dedupe_key,
        )
    }

    /// One alert per (budget, month, threshold): the `finance_budget_alerts` row
    /// already guarantees a single enqueue, the key is the second line of defence.
    pub fn budget_alert(
        owner: AccountId,
        budget_id: i64,
        month: Month,
        threshold: Threshold,
    ) -> Self {
        let pct = threshold.pct();
        let key = format!("budget:{budget_id}:{month}:{pct}");
        let payload = JobPayload::BudgetAlert(BudgetAlertPayload {
            budget_id,
            month,
            threshold: pct,
        });
        Self::new(JobType::BudgetAlertSend, Some(owner), payload, 3, 60, key)
    }

    pub fn with_priority(mut self, p: i16) -> Self {
        self.priority = p;
        self
    }

    pub fn job_type(&self) -> JobType {
        self.job_type
    }
    pub fn account_id(&self) -> Option<&AccountId> {
        self.account_id.as_ref()
    }
    pub fn payload(&self) -> &JobPayload {
        &self.payload
    }
    pub fn priority(&self) -> i16 {
        self.priority
    }
    pub fn max_attempts(&self) -> i32 {
        self.max_attempts
    }
    pub fn backoff_seconds(&self) -> i32 {
        self.backoff_seconds
    }
    pub fn available_at(&self) -> Option<DateTime<Utc>> {
        self.available_at
    }
    pub fn dedupe_key(&self) -> Option<&str> {
        self.dedupe_key.as_deref()
    }
}

#[cfg(test)]
impl NewJob {
    pub fn test_job(job_type: JobType, dedupe_key: Option<&str>) -> Self {
        Self {
            job_type,
            account_id: None,
            payload: JobPayload::Empty,
            priority: 0,
            max_attempts: 3,
            backoff_seconds: 0,
            available_at: None,
            dedupe_key: dedupe_key.map(str::to_owned),
        }
    }
    pub fn with_available_at(mut self, at: DateTime<Utc>) -> Self {
        self.available_at = Some(at);
        self
    }
    pub fn with_policy(mut self, max_attempts: i32, backoff_seconds: i32) -> Self {
        self.max_attempts = max_attempts;
        self.backoff_seconds = backoff_seconds;
        self
    }
    pub fn for_account(mut self, owner: AccountId) -> Self {
        self.account_id = Some(owner);
        self
    }
}

/// Delay before retry number `attempt + 1`: `backoff · 2^(attempt-1)`,
/// attempt counted from 1, capped at [`MAX_BACKOFF`].
pub fn backoff_delay(backoff_seconds: i32, attempt: i32) -> Duration {
    let base = u64::try_from(backoff_seconds.max(0)).unwrap_or(0);
    let exponent = u32::try_from((attempt.max(1) - 1).min(20)).unwrap_or(20);
    let secs = base.saturating_mul(1u64 << exponent);
    Duration::from_secs(secs).min(MAX_BACKOFF)
}

/// Error text as stored on a job row: at most [`MAX_ERROR_CHARS`] characters.
pub fn truncate_error(message: &str) -> String {
    message.chars().take(MAX_ERROR_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn owner() -> AccountId {
        AccountId::from_seed("acct_alice")
    }

    #[test]
    fn job_type_round_trips_through_its_wire_name() {
        for t in JobType::ALL {
            assert_eq!(JobType::parse(t.as_str()), Some(t));
        }
        assert_eq!(JobType::ReminderFire.as_str(), "reminder.fire");
        assert_eq!(JobType::BriefingSend.as_str(), "briefing.send");
        assert_eq!(
            JobType::PortfolioCheckAlerts.as_str(),
            "portfolio.check_alerts"
        );
        assert_eq!(
            JobType::PortfolioRefreshPrices.as_str(),
            "portfolio.refresh_prices"
        );
        assert_eq!(JobType::parse("nope"), None);
    }

    #[test]
    fn job_status_round_trips() {
        for s in ["pending", "running", "completed", "failed", "cancelled"] {
            assert_eq!(JobStatus::parse(s).map(JobStatus::as_str), Some(s));
        }
        assert_eq!(JobStatus::parse("bogus"), None);
    }

    /// Given the same reminder and trigger, When built twice, Then the dedupe
    /// key is identical (a double tick enqueues once); a moved trigger_at
    /// (snooze, next recurrence) yields a new key.
    #[test]
    fn reminder_fire_key_is_stable_per_trigger() {
        let t = Utc.with_ymd_and_hms(2026, 9, 29, 7, 30, 0).unwrap();
        let a = NewJob::reminder_fire(owner(), 42, t);
        let b = NewJob::reminder_fire(owner(), 42, t);
        assert_eq!(a.dedupe_key(), Some("42:2026-09-29T07:30:00.000000Z"));
        assert_eq!(a.dedupe_key(), b.dedupe_key());
        let later = NewJob::reminder_fire(owner(), 42, t + chrono::Duration::days(1));
        assert_ne!(a.dedupe_key(), later.dedupe_key());
        assert_eq!((a.max_attempts(), a.backoff_seconds()), (5, 30));
        assert_eq!(a.job_type(), JobType::ReminderFire);
        assert_eq!(a.account_id(), Some(&owner()));
    }

    #[test]
    fn new_job_budget_alert_dedupe_key() {
        let month = Month::parse("2026-10").unwrap();
        let job = NewJob::budget_alert(owner(), 7, month, Threshold::Warning);
        assert_eq!(job.dedupe_key(), Some("budget:7:2026-10:80"));
        assert_eq!(job.job_type().as_str(), "finance.budget_alert");
        assert_eq!((job.max_attempts(), job.backoff_seconds()), (3, 60));
        assert_eq!(job.account_id(), Some(&owner()));
        assert_eq!(
            job.payload(),
            &JobPayload::BudgetAlert(BudgetAlertPayload {
                budget_id: 7,
                month,
                threshold: 80
            })
        );
        let exceeded = NewJob::budget_alert(owner(), 7, month, Threshold::Exceeded);
        assert_eq!(exceeded.dedupe_key(), Some("budget:7:2026-10:100"));
    }

    #[test]
    fn briefing_key_is_account_and_local_date() {
        let d = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let job = NewJob::briefing_send(owner(), d);
        assert_eq!(
            job.dedupe_key(),
            Some(format!("{}:2026-09-29", owner()).as_str())
        );
        assert_eq!((job.max_attempts(), job.backoff_seconds()), (3, 60));
    }

    #[test]
    fn alert_check_key_buckets_by_unix_minute_and_has_no_account() {
        let t = Utc.with_ymd_and_hms(2026, 9, 29, 7, 30, 59).unwrap();
        let job = NewJob::portfolio_check_alerts(t);
        assert_eq!(
            job.dedupe_key(),
            Some(format!("alerts:{}", t.timestamp() / 60).as_str())
        );
        assert_eq!(job.account_id(), None);
        assert_eq!(job.max_attempts(), 1);
        let same_minute = NewJob::portfolio_check_alerts(t - chrono::Duration::seconds(30));
        assert_eq!(job.dedupe_key(), same_minute.dedupe_key());
    }

    #[test]
    fn price_refresh_key_is_account_and_utc_hour() {
        let t = Utc.with_ymd_and_hms(2026, 9, 29, 7, 59, 0).unwrap();
        let job = NewJob::portfolio_refresh_prices(owner(), t);
        assert_eq!(
            job.dedupe_key(),
            Some(format!("{}:2026-09-29T07", owner()).as_str())
        );
        assert_eq!((job.max_attempts(), job.backoff_seconds()), (3, 60));
    }

    /// R21: 30/60/120/240 s for the reminder policy.
    #[test]
    fn backoff_doubles_per_attempt_from_the_base() {
        let secs = |a| backoff_delay(30, a).as_secs();
        assert_eq!([secs(1), secs(2), secs(3), secs(4)], [30, 60, 120, 240]);
        assert_eq!(
            backoff_delay(30, 0).as_secs(),
            30,
            "attempt 0 is treated as the first"
        );
        assert_eq!(backoff_delay(0, 5).as_secs(), 0);
        assert_eq!(
            backoff_delay(i32::MAX, i32::MAX),
            MAX_BACKOFF,
            "never overflows"
        );
    }

    #[test]
    fn truncate_error_keeps_whole_chars_up_to_the_limit() {
        let long = "é".repeat(MAX_ERROR_CHARS + 10);
        let cut = truncate_error(&long);
        assert_eq!(cut.chars().count(), MAX_ERROR_CHARS);
        assert_eq!(truncate_error("short"), "short");
    }

    #[test]
    fn payloads_serialise_as_plain_objects() {
        let t = Utc.with_ymd_and_hms(2026, 9, 29, 7, 30, 0).unwrap();
        let p = ReminderFirePayload {
            reminder_id: 7,
            trigger_at: t,
        };
        let back: ReminderFirePayload =
            serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn media_process_receipt_builds_and_dedupes_by_storage_key() {
        let o = owner();
        let key = crate::domain::storage::StorageKey::generate(
            &o,
            crate::domain::storage::StorageCategory::Receipts,
            "jpg",
        );
        let conv = Some(ConversationAddress::Matrix("!room:example.org".into()));
        let job = NewJob::media_process_receipt(o, key.clone(), conv.clone(), Some("lunch".into()));

        assert_eq!(job.job_type(), JobType::MediaProcessReceipt);
        assert_eq!(job.account_id(), Some(&o));
        assert_eq!(job.dedupe_key(), Some(key.as_str()));
        assert_eq!((job.max_attempts(), job.backoff_seconds()), (3, 60));

        let payload = match job.payload() {
            JobPayload::MediaProcessReceipt(p) => p,
            _ => panic!("wrong payload variant"),
        };
        assert_eq!(payload.storage_key, key);
        assert_eq!(payload.account_id, o);
        assert_eq!(payload.conversation, conv);
        assert_eq!(payload.caption.as_deref(), Some("lunch"));

        // Payload serde round-trip
        let val = serde_json::to_value(payload).unwrap();
        let back: ProcessReceiptPayload = serde_json::from_value(val).unwrap();
        assert_eq!(&back, payload);
    }

    #[test]
    fn knowledge_embed_item_builds_and_dedupes_by_item_type_and_id() {
        let o = owner();
        let job = NewJob::knowledge_embed_item(o, KnowledgeItemType::Note, 101);
        assert_eq!(job.job_type(), JobType::KnowledgeEmbedItem);
        assert_eq!(job.account_id(), Some(&o));
        assert_eq!(job.dedupe_key(), Some("note:101"));
        assert_eq!((job.max_attempts(), job.backoff_seconds()), (3, 30));

        let payload = match job.payload() {
            JobPayload::KnowledgeEmbed(p) => p,
            _ => panic!("wrong payload variant"),
        };
        assert_eq!(payload.account_id, o);
        assert_eq!(payload.item_type, KnowledgeItemType::Note);
        assert_eq!(payload.item_id, 101);

        let val = serde_json::to_value(payload).unwrap();
        let back: KnowledgeEmbedPayload = serde_json::from_value(val).unwrap();
        assert_eq!(&back, payload);

        let now = Utc.with_ymd_and_hms(2026, 10, 9, 14, 30, 0).unwrap();
        let backfill = NewJob::knowledge_backfill_embeddings(now);
        assert_eq!(backfill.job_type(), JobType::KnowledgeBackfillEmbeddings);
        assert_eq!(backfill.account_id(), None);
        assert_eq!(
            backfill.dedupe_key(),
            Some("knowledge:backfill:2026-10-09T14")
        );
        let same_hour = now + chrono::Duration::minutes(20);
        assert_eq!(
            NewJob::knowledge_backfill_embeddings(same_hour).dedupe_key(),
            backfill.dedupe_key()
        );
        let next_hour = now + chrono::Duration::hours(1);
        assert_ne!(
            NewJob::knowledge_backfill_embeddings(next_hour).dedupe_key(),
            backfill.dedupe_key()
        );
    }

    #[test]
    fn a_new_version_of_an_item_gets_its_own_embed_key() {
        let o = owner();
        let v1 = NewJob::knowledge_embed_item_versioned(o, KnowledgeItemType::Note, 7, "100");
        let v2 = NewJob::knowledge_embed_item_versioned(o, KnowledgeItemType::Note, 7, "200");
        assert_eq!(v1.dedupe_key(), Some("note:7:100"));
        assert_ne!(v1.dedupe_key(), v2.dedupe_key());
        assert_eq!(
            v1.dedupe_key(),
            NewJob::knowledge_embed_item_versioned(o, KnowledgeItemType::Note, 7, "100")
                .dedupe_key(),
            "the same version stays deduplicated"
        );
        assert_eq!(v1.job_type(), JobType::KnowledgeEmbedItem);
        assert_eq!((v1.max_attempts(), v1.backoff_seconds()), (3, 30));
    }
}
