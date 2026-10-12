//! Application service for travel footprints.
//!
//! Orchestrates footprint operations, input validation, settings lookup,
//! and system event publication.

use sqlx::PgPool;

use crate::application::settings::SettingsService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::finance::money::{exponent_for, normalise_currency, parse_amount, MoneyError};
use crate::domain::footprints::{
    calculate_stay_duration, CreateFootprint, Footprint, FootprintsSummary, UpdateFootprint,
    MAX_NOTE_LEN, MAX_TITLE_LEN,
};
use crate::infrastructure::postgres::repositories::{self, outbox};

#[derive(Debug, Clone)]
pub struct CreateFootprintInput {
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: String,
    pub departure_date: Option<String>,
    pub spend_amount: Option<String>,
    pub spend_currency: Option<String>,
    pub rating: Option<i64>,
    pub category: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateFootprintInput {
    pub title: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: Option<String>,
    pub departure_date: Option<String>,
    pub spend_amount: Option<String>,
    pub spend_currency: Option<String>,
    pub rating: Option<i64>,
    pub category: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone)]
pub struct FootprintService {
    pool: PgPool,
    settings: SettingsService,
}

impl FootprintService {
    pub fn new(pool: PgPool, settings: SettingsService) -> Self {
        Self { pool, settings }
    }

    pub async fn create(
        &self,
        owner: &AccountId,
        input: CreateFootprintInput,
    ) -> Result<Footprint, AppError> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(AppError::Validation(
                "Title / Place Name is required".to_string(),
            ));
        }
        if title.len() > MAX_TITLE_LEN {
            return Err(AppError::Validation(
                "Title exceeds 500 characters".to_string(),
            ));
        }

        if let Some(ref note) = input.note {
            if note.len() > MAX_NOTE_LEN {
                return Err(AppError::Validation(
                    "Note exceeds 500 characters".to_string(),
                ));
            }
        }

        let default_currency = self.settings.get_or_default(owner).await.currency;
        let raw_currency = input.spend_currency.as_deref().unwrap_or(&default_currency);
        let currency =
            normalise_currency(raw_currency).map_err(|e| AppError::Validation(e.to_string()))?;
        let exp = exponent_for(&currency);

        let spend_minor = if let Some(ref s) = input.spend_amount {
            parse_spend_amount(s, exp)?
        } else {
            0
        };

        let rating = input.rating.unwrap_or(5).clamp(1, 5);
        let category = input.category.unwrap_or_else(|| "travel".to_string());
        let duration =
            calculate_stay_duration(&input.arrival_date, input.departure_date.as_deref());

        let cmd = CreateFootprint {
            title: title.to_string(),
            latitude: input.latitude,
            longitude: input.longitude,
            country: input.country,
            city: input.city,
            address: input.address,
            arrival_date: input.arrival_date,
            departure_date: input.departure_date,
            stay_duration_days: duration,
            spend_amount_minor: spend_minor,
            spend_currency: currency,
            spend_currency_exponent: exp as i64,
            rating,
            category,
            note: input.note,
        };

        let mut tx = self.pool.begin().await?;
        let created = repositories::footprints::create(&mut *tx, owner, &cmd).await?;
        outbox::publish(&mut tx, &SystemEvent::FootprintCreated(created.clone())).await?;
        tx.commit().await?;
        Ok(created)
    }

    pub async fn get(&self, owner: &AccountId, id: i64) -> Result<Footprint, AppError> {
        repositories::footprints::find_owned(&self.pool, id, owner)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Footprint #{id} not found")))
    }

    pub async fn list(
        &self,
        owner: &AccountId,
        country: Option<&str>,
        category: Option<&str>,
        search: Option<&str>,
    ) -> Result<Vec<Footprint>, AppError> {
        let items =
            repositories::footprints::list_by_owner(&self.pool, owner, country, category, search)
                .await?;
        Ok(items)
    }

    pub async fn update(
        &self,
        owner: &AccountId,
        id: i64,
        input: UpdateFootprintInput,
    ) -> Result<Footprint, AppError> {
        let existing = self.get(owner, id).await?;

        let title = match input.title.as_deref() {
            Some(t) => {
                let trimmed = t.trim();
                if trimmed.is_empty() {
                    return Err(AppError::Validation(
                        "Title / Place Name cannot be empty".to_string(),
                    ));
                }
                if trimmed.len() > MAX_TITLE_LEN {
                    return Err(AppError::Validation(
                        "Title exceeds 500 characters".to_string(),
                    ));
                }
                trimmed.to_string()
            }
            None => existing.title,
        };

        let note = match input.note {
            Some(n) => {
                if n.len() > MAX_NOTE_LEN {
                    return Err(AppError::Validation(
                        "Note exceeds 500 characters".to_string(),
                    ));
                }
                Some(n)
            }
            None => existing.note,
        };

        let lat = input.latitude.unwrap_or(existing.latitude);
        let lon = input.longitude.unwrap_or(existing.longitude);
        let country = input.country.or(existing.country);
        let city = input.city.or(existing.city);
        let address = input.address.or(existing.address);
        let arrival_date = input.arrival_date.unwrap_or(existing.arrival_date);
        let departure_date = input.departure_date.or(existing.departure_date);

        let raw_currency = input
            .spend_currency
            .as_deref()
            .unwrap_or(&existing.spend_currency);
        let currency =
            normalise_currency(raw_currency).map_err(|e| AppError::Validation(e.to_string()))?;
        let exp = exponent_for(&currency);

        let spend_minor = if let Some(ref s) = input.spend_amount {
            parse_spend_amount(s, exp)?
        } else if exp as i64 != existing.spend_currency_exponent && existing.spend_amount_minor > 0
        {
            let formatted = crate::domain::finance::money::format_amount(
                existing.spend_amount_minor,
                existing.spend_currency_exponent as u32,
            );
            parse_spend_amount(&formatted, exp)?
        } else {
            existing.spend_amount_minor
        };

        let rating = input.rating.unwrap_or(existing.rating).clamp(1, 5);
        let category = input.category.unwrap_or(existing.category);
        let duration = calculate_stay_duration(&arrival_date, departure_date.as_deref());

        let cmd = UpdateFootprint {
            title,
            latitude: lat,
            longitude: lon,
            country,
            city,
            address,
            arrival_date,
            departure_date,
            stay_duration_days: duration,
            spend_amount_minor: spend_minor,
            spend_currency: currency,
            spend_currency_exponent: exp as i64,
            rating,
            category,
            note,
        };

        let mut tx = self.pool.begin().await?;
        let updated = repositories::footprints::update(&mut *tx, id, owner, &cmd).await?;
        let updated =
            updated.ok_or_else(|| AppError::NotFound(format!("Footprint #{id} not found")))?;
        outbox::publish(&mut tx, &SystemEvent::FootprintUpdated(updated.clone())).await?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn delete(&self, owner: &AccountId, id: i64) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        let deleted = repositories::footprints::delete(&mut *tx, id, owner).await?;
        if !deleted {
            return Err(AppError::NotFound(format!("Footprint #{id} not found")));
        }
        outbox::publish(
            &mut tx,
            &SystemEvent::FootprintDeleted {
                id,
                owner_id: *owner,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn summary_stats(&self, owner: &AccountId) -> Result<FootprintsSummary, AppError> {
        let stats = repositories::footprints::summary_stats(&self.pool, owner).await?;
        Ok(stats)
    }
}

/// Parses a human-typed spend amount into integer minor units.
/// Unlike financial transactions, footprint spending is optional and can be zero ($0).
fn parse_spend_amount(input: &str, exp: u32) -> Result<i64, AppError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    match parse_amount(trimmed, exp) {
        Ok(minor) => Ok(minor),
        Err(MoneyError::NotPositive) => {
            let cleaned: String = trimmed
                .trim_start_matches(['$', '€', '£', '¥', '₹', '₩'])
                .chars()
                .filter(|c| *c != ',' && *c != '_' && !c.is_whitespace() && *c != '.')
                .collect();
            if !cleaned.is_empty() && cleaned.chars().all(|c| c == '0') {
                Ok(0)
            } else {
                Err(AppError::Validation(
                    "Spend amount cannot be negative".to_string(),
                ))
            }
        }
        Err(e) => Err(AppError::Validation(e.to_string())),
    }
}
