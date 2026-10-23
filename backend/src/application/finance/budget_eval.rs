//! Budget threshold evaluation, run inside the transaction of every expense write so
//! the ledger row, the alert rows, the event and the delivery job commit or roll back
//! together (spec §5.3).

use chrono::Utc;
use sqlx::Postgres;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::finance::budget::{
    reached_thresholds, status, BudgetState, BudgetStatus, Month, Threshold,
};
use crate::domain::finance::{Direction, Transaction};
use crate::domain::jobs::NewJob;
use crate::infrastructure::postgres::repositories::finance::budgets as repo;
use crate::infrastructure::postgres::repositories::{jobs, outbox, settings};

use super::budgets::spent_for;
use super::FinanceService;

impl FinanceService {
    /// Measures the alert-enabled budgets `tx` counts toward against the owner's
    /// current month and records every newly reached threshold. Per budget it
    /// publishes one `BudgetThresholdCrossed` and enqueues one job, both for the
    /// highest threshold inserted by this call. Returns the evaluated budgets
    /// that are not `Ok`.
    ///
    /// Only an expense dated inside the owner's current month is evaluated; a
    /// backdated or income row never alerts. The matching budgets are row-locked
    /// before the month is summed, so two concurrent expenses are evaluated one
    /// after the other and the second sees the first's committed spend.
    pub async fn evaluate_budgets_in_tx(
        &self,
        dbtx: &mut sqlx::Transaction<'_, Postgres>,
        owner: &AccountId,
        tx: &Transaction,
    ) -> Result<Vec<BudgetStatus>, AppError> {
        if tx.direction != Direction::Expense {
            return Ok(Vec::new());
        }
        let tz = settings::get(&mut **dbtx, owner)
            .await?
            .unwrap_or_default()
            .tz();
        let now = Utc::now();
        let month = Month::containing(now, tz);
        if Month::containing(tx.occurred_at, tz) != month {
            return Ok(Vec::new());
        }

        let budgets = repo::lock_matching(&mut **dbtx, owner, &tx.currency, tx.category_id).await?;
        if budgets.is_empty() {
            return Ok(Vec::new());
        }
        let rows = repo::month_spend(
            &mut **dbtx,
            owner,
            month.start_utc(tz),
            month.end_utc(tz),
            Some(&tx.currency),
        )
        .await?;

        let mut flagged = Vec::new();
        for budget in budgets {
            let spent = spent_for(&budget, &rows);
            let mut highest: Option<Threshold> = None;
            for threshold in reached_thresholds(spent, budget.limit_minor) {
                if repo::record_alert(&mut **dbtx, budget.id, month, threshold).await? {
                    highest = Some(threshold);
                }
            }
            if let Some(threshold) = highest {
                outbox::publish(
                    dbtx,
                    &SystemEvent::BudgetThresholdCrossed {
                        owner_id: *owner,
                        budget_id: budget.id,
                        month,
                        threshold: threshold.pct(),
                        spent_minor: spent,
                        limit_minor: budget.limit_minor,
                        currency: budget.currency.clone(),
                        currency_exponent: budget.currency_exponent,
                        scope_name: budget.scope.label().to_string(),
                    },
                )
                .await?;
                let job = NewJob::budget_alert(*owner, budget.id, month, threshold);
                jobs::enqueue(&mut **dbtx, &job).await?;
            }
            let measured = status(budget, spent, month, now, tz);
            if measured.state != BudgetState::Ok {
                flagged.push(measured);
            }
        }
        Ok(flagged)
    }
}
