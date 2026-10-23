//! `finance.budget_alert`: DMs the owner that a budget crossed 80 % or 100 % of
//! its monthly limit. The alert row is already recorded by the evaluation that
//! enqueued the job, so a retry here only re-sends; no business rules.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Utc};

use super::dm::{dm_linked_identities, owner_ready};
use super::{parse_payload, Outbound, CHANNEL_RETRY};
use crate::application::AppServices;
use crate::domain::finance::budget::{BudgetStatus, Threshold};
use crate::domain::finance::money::format_amount;
use crate::domain::jobs::{BudgetAlertPayload, JobType};
use crate::jobs::{JobContext, JobError, JobHandler};
use crate::utils::html::html_escape;

pub struct BudgetAlertHandler {
    services: AppServices,
    outbound: Arc<dyn Outbound>,
}

impl BudgetAlertHandler {
    pub fn new(services: AppServices, outbound: Arc<dyn Outbound>) -> Self {
        Self { services, outbound }
    }
}

/// Plain and HTML copy for a crossed threshold. `today` is the owner's local
/// date; today counts as used when reporting the days left.
pub(crate) fn format_budget_alert(
    s: &BudgetStatus,
    threshold: Threshold,
    today: NaiveDate,
) -> (String, String) {
    let exp = u32::try_from(s.budget.currency_exponent).unwrap_or(0);
    let spent = format!(
        "{} {}",
        format_amount(s.spent_minor, exp),
        s.budget.currency
    );
    let limit = format!(
        "{} {}",
        format_amount(s.budget.limit_minor, exp),
        s.budget.currency
    );
    let label = s.budget.scope.label();
    let (head, tail) = match threshold {
        Threshold::Warning => {
            let first = s.month.first_day();
            let left = if today.year() == first.year() && today.month() == first.month() {
                s.month.days().saturating_sub(today.day())
            } else {
                0
            };
            let unit = if left == 1 { "day" } else { "days" };
            (
                "Budget warning",
                format!(
                    "{spent} of {limit} used ({} %) with {left} {unit} left.",
                    s.percent_used
                ),
            )
        }
        Threshold::Exceeded => (
            "Budget exceeded",
            format!("{spent} of {limit} ({} %).", s.percent_used),
        ),
    };
    let plain = format!("{head}: {label} — {tail}");
    let html = format!(
        "{head}: <b>{}</b> — {}",
        html_escape(label),
        html_escape(&tail)
    );
    (plain, html)
}

#[async_trait]
impl JobHandler for BudgetAlertHandler {
    fn job_type(&self) -> JobType {
        JobType::BudgetAlertSend
    }

    async fn run(&self, ctx: &JobContext, payload: serde_json::Value) -> Result<(), JobError> {
        let owner = ctx.account_id.ok_or_else(|| {
            JobError::Permanent(anyhow::anyhow!("budget alert without an account"))
        })?;
        let payload: BudgetAlertPayload = parse_payload(payload)?;
        let threshold = Threshold::from_pct(payload.threshold).ok_or_else(|| {
            JobError::Permanent(anyhow::anyhow!("unknown threshold {}", payload.threshold))
        })?;
        if !owner_ready(&self.services, self.outbound.as_ref(), &owner).await? {
            return Err(JobError::Unavailable(CHANNEL_RETRY));
        }
        let Some(status) = self
            .services
            .finance
            .status_for_budget(&owner, payload.budget_id, payload.month)
            .await?
        else {
            return Ok(());
        };
        let tz = self.services.settings.get_or_default(&owner).await.tz();
        let today = Utc::now().with_timezone(&tz).date_naive();
        let (plain, html) = format_budget_alert(&status, threshold, today);
        let identities = self.services.identity.list(&owner).await?;
        dm_linked_identities(self.outbound.as_ref(), identities, &plain, &html).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone, Utc};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::finance::budget::{status, Budget, BudgetScope, Month, NewBudget};
    use crate::domain::finance::{Direction, NewTransaction};
    use crate::domain::jobs::JobId;
    use crate::interface::jobs::fake::FakeOutbound;
    use crate::utils::time::now_at_storage_precision;

    fn status_of(scope: BudgetScope, spent: i64, limit: i64) -> BudgetStatus {
        let at = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let budget = Budget {
            id: 1,
            owner_id: AccountId::generate(),
            scope,
            currency: "EUR".into(),
            currency_exponent: 2,
            limit_minor: limit,
            alerts_enabled: true,
            created_at: at,
            updated_at: at,
        };
        let month = Month::parse("2026-10").unwrap();
        status(budget, spent, month, at, chrono_tz::Tz::UTC)
    }

    fn category(name: &str) -> BudgetScope {
        BudgetScope::Category {
            id: 1,
            name: name.into(),
        }
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn format_warning_matches_spec_copy() {
        let s = status_of(category("Food"), 24_600, 30_000);
        let (plain, html) = format_budget_alert(&s, Threshold::Warning, day(19));
        assert_eq!(
            plain,
            "Budget warning: Food — 246.00 EUR of 300.00 EUR used (82 %) with 12 days left."
        );
        assert!(html.contains("Food"));
    }

    #[test]
    fn format_exceeded_matches_spec_copy() {
        let s = status_of(BudgetScope::Overall, 153_000, 150_000);
        let (plain, _) = format_budget_alert(&s, Threshold::Exceeded, day(19));
        assert_eq!(
            plain,
            "Budget exceeded: Overall — 1530.00 EUR of 1500.00 EUR (102 %)."
        );
    }

    #[test]
    fn format_escapes_category_name() {
        let s = status_of(category("<b>x</b>"), 24_600, 30_000);
        let (_, html) = format_budget_alert(&s, Threshold::Warning, day(19));
        assert!(html.contains("&lt;b&gt;x&lt;/b&gt;"), "{html}");
        assert!(!html.contains("<b>x</b>"));
    }

    async fn setup(name: &str) -> (AppServices, AccountId, i64) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = services
            .identity
            .provision_matrix_for_test(&format!("@{name}:test"))
            .await;
        let budget = services
            .finance
            .create_budget(
                &owner,
                NewBudget::new(None, "EUR", 10_000, Some(true)).unwrap(),
            )
            .await
            .unwrap();
        services
            .finance
            .record_transaction(
                &owner,
                NewTransaction {
                    direction: Direction::Expense,
                    amount_minor: 8_200,
                    currency: "EUR",
                    currency_exponent: 2,
                    category: None,
                    note: None,
                    receipt_storage_key: None,
                    room_id: "web",
                    sender_name: "tester",
                    occurred_at: now_at_storage_precision(),
                    source: None,
                    create_missing_category: false,
                },
            )
            .await
            .unwrap();
        (services, owner, budget.id)
    }

    fn ctx(owner: AccountId) -> JobContext {
        JobContext {
            job_id: JobId(1),
            account_id: Some(owner),
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        }
    }

    fn payload(budget_id: i64) -> serde_json::Value {
        let month = Month::containing(Utc::now(), chrono_tz::Tz::UTC);
        serde_json::json!({"budget_id": budget_id, "month": month.to_string(), "threshold": 80})
    }

    #[tokio::test]
    async fn sends_to_every_connected_identity() {
        let (s, owner, id) = setup("bud_dm").await;
        let fake = FakeOutbound::connected();
        let handler = BudgetAlertHandler::new(s, fake.clone());

        let out = handler.run(&ctx(owner), payload(id)).await;

        assert!(out.is_ok(), "{out:?}");
        let dms = fake.dms();
        assert_eq!(dms.len(), 1);
        assert_eq!(dms[0].0, "@bud_dm:test");
        assert!(
            dms[0].1.starts_with("Budget warning: Overall"),
            "{}",
            dms[0].1
        );
    }

    #[tokio::test]
    async fn defers_while_disconnected() {
        let (s, owner, id) = setup("bud_off").await;
        let fake = FakeOutbound::disconnected();
        let handler = BudgetAlertHandler::new(s, fake.clone());

        let out = handler.run(&ctx(owner), payload(id)).await;

        assert!(
            matches!(out, Err(JobError::Unavailable(d)) if d == CHANNEL_RETRY),
            "{out:?}"
        );
        assert!(fake.dms().is_empty());
    }

    #[tokio::test]
    async fn deleted_budget_completes_silently() {
        let (s, owner, id) = setup("bud_gone").await;
        s.finance.delete_budget(&owner, id).await.unwrap();
        let fake = FakeOutbound::connected();
        let handler = BudgetAlertHandler::new(s, fake.clone());

        let out = handler.run(&ctx(owner), payload(id)).await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.dms().is_empty());
    }
}
