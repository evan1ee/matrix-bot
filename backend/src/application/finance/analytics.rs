use chrono::{DateTime, Datelike, Utc};
use uuid::Uuid;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::finance::analytics::{
    ExpenseCategorySlice, FinanceAnalyticsReport, MonthlyCashflowBucket, PublicFinanceReport,
};
use crate::domain::finance::money::exponent_for;
use crate::domain::finance::{CategoryTotal, CurrencySummary, FinanceError, FinanceShare};
use crate::infrastructure::postgres::repositories::finance as repo;

use super::FinanceService;

impl FinanceService {
    pub async fn summary(
        &self,
        owner: &AccountId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CurrencySummary>, AppError> {
        let res = repo::summary(&self.pool, owner, from, to).await?;
        Ok(res)
    }

    pub async fn get_category_summary(
        &self,
        owner: &AccountId,
        category_id: Option<i64>,
        currency: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Option<CategoryTotal>, AppError> {
        let res =
            repo::get_category_summary(&self.pool, owner, category_id, currency, since, until)
                .await?;
        Ok(res)
    }

    pub async fn get_analytics_report(
        &self,
        owner: &AccountId,
        currency_opt: Option<&str>,
        months_limit: i64,
    ) -> Result<FinanceAnalyticsReport, AppError> {
        let currency = match currency_opt {
            Some(c) if !c.trim().is_empty() => c.trim().to_uppercase(),
            _ => repo::primary_currency(&self.pool, owner).await?,
        };
        let exponent = exponent_for(&currency) as i64;
        let months = months_limit.clamp(1, 36);

        let now = Utc::now();
        let current_year = now.year();
        let current_month = now.month();

        let mut month_keys: Vec<String> = Vec::with_capacity(months as usize);
        for i in (0..months).rev() {
            let total_months = (current_year * 12 + (current_month as i32 - 1)) - i as i32;
            let y = total_months / 12;
            let m = (total_months % 12) + 1;
            month_keys.push(format!("{y:04}-{m:02}"));
        }

        let earliest_month = month_keys
            .first()
            .cloned()
            .unwrap_or_else(|| "1970-01".into());

        let cutoff_start =
            chrono::NaiveDate::parse_from_str(&format!("{earliest_month}-01"), "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
                .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid"));

        let cashflow_rows =
            repo::monthly_cashflow_rows(&self.pool, owner, &currency, cutoff_start).await?;

        let mut monthly_cashflow: Vec<MonthlyCashflowBucket> = month_keys
            .iter()
            .map(|k| MonthlyCashflowBucket {
                month: k.clone(),
                income_minor: 0,
                expense_minor: 0,
                net_minor: 0,
                currency: currency.clone(),
                currency_exponent: exponent,
            })
            .collect();

        for row in cashflow_rows {
            if let Some(month_str) = row.month_str {
                if let Some(bucket) = monthly_cashflow.iter_mut().find(|b| b.month == month_str) {
                    let total = row.total_minor.unwrap_or(0);
                    if row.direction == "expense" {
                        bucket.expense_minor += total;
                    } else if row.direction == "income" {
                        bucket.income_minor += total;
                    }
                    bucket.net_minor = bucket.income_minor - bucket.expense_minor;
                }
            }
        }

        let total_income: i64 = monthly_cashflow.iter().map(|b| b.income_minor).sum();
        let total_expense: i64 = monthly_cashflow.iter().map(|b| b.expense_minor).sum();
        let net_savings = total_income - total_expense;

        let cat_rows =
            repo::category_slice_rows(&self.pool, owner, &currency, cutoff_start).await?;

        let expense_categories: Vec<ExpenseCategorySlice> = cat_rows
            .into_iter()
            .map(|r| {
                let amount = r.total_minor.unwrap_or(0);
                let percentage = if total_expense > 0 {
                    (amount as f64 / total_expense as f64) * 100.0
                } else {
                    0.0
                };
                ExpenseCategorySlice {
                    category_name: r.category_name.unwrap_or_else(|| "Uncategorized".into()),
                    amount_minor: amount,
                    percentage,
                    tx_count: r.tx_count.unwrap_or(0),
                    currency: currency.clone(),
                    currency_exponent: exponent,
                }
            })
            .collect();

        Ok(FinanceAnalyticsReport {
            currency,
            currency_exponent: exponent,
            months_requested: months,
            total_income_minor: total_income,
            total_expense_minor: total_expense,
            net_savings_minor: net_savings,
            monthly_cashflow,
            expense_categories,
        })
    }

    // ---- Public Sharing ----------------------------------------------------

    pub async fn create_share(
        &self,
        owner: &AccountId,
        title: Option<&str>,
    ) -> Result<FinanceShare, AppError> {
        let token = Uuid::new_v4().to_string();
        let share = repo::create_share(&self.pool, owner, &token, title).await?;
        Ok(share)
    }

    pub async fn upsert_share(
        &self,
        owner: &AccountId,
        is_enabled: bool,
        title: Option<&str>,
    ) -> Result<FinanceShare, AppError> {
        let existing = self.get_share(owner).await?;
        let token = existing
            .map(|s| s.token)
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());

        let share = repo::upsert_share(&self.pool, owner, &token, is_enabled, title).await?;
        Ok(share)
    }

    pub async fn disable_share(&self, owner: &AccountId) -> Result<(), AppError> {
        repo::toggle_share(&self.pool, owner, false).await?;
        Ok(())
    }

    pub async fn get_share(&self, owner: &AccountId) -> Result<Option<FinanceShare>, AppError> {
        let share = repo::get_share(&self.pool, owner).await?;
        Ok(share)
    }

    pub async fn toggle_share(
        &self,
        owner: &AccountId,
        is_enabled: bool,
    ) -> Result<FinanceShare, AppError> {
        let share = repo::toggle_share(&self.pool, owner, is_enabled)
            .await?
            .ok_or(FinanceError::ShareNotFound)?;
        Ok(share)
    }

    pub async fn update_share_title(
        &self,
        owner: &AccountId,
        title: Option<&str>,
    ) -> Result<FinanceShare, AppError> {
        let share = repo::update_share_title(&self.pool, owner, title)
            .await?
            .ok_or(FinanceError::ShareNotFound)?;
        Ok(share)
    }

    pub async fn get_public_report(
        &self,
        token: &str,
        months_limit: i64,
    ) -> Result<Option<PublicFinanceReport>, AppError> {
        let share = match repo::find_share_by_token(&self.pool, token).await? {
            Some(s) if s.is_enabled => s,
            _ => return Ok(None),
        };

        let report = self
            .get_analytics_report(&share.owner_id, None, months_limit)
            .await?;

        let days = months_limit.clamp(1, 36) * 31;
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let recent_transactions = repo::recent_public_transactions(
            &self.pool,
            &share.owner_id,
            &report.currency,
            cutoff,
            50,
        )
        .await?;

        Ok(Some(PublicFinanceReport {
            title: share
                .title
                .unwrap_or_else(|| "Public Transparency Ledger".into()),
            currency: report.currency,
            currency_exponent: report.currency_exponent,
            total_income_minor: report.total_income_minor,
            total_expense_minor: report.total_expense_minor,
            net_savings_minor: report.net_savings_minor,
            monthly_cashflow: report.monthly_cashflow,
            expense_categories: report.expense_categories,
            recent_transactions,
        }))
    }
}
