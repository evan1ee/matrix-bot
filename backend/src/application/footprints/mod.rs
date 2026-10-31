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
use crate::domain::jobs::NewJob;
use crate::domain::knowledge::search::KnowledgeItemType;
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
        repositories::jobs::enqueue(&mut *tx, &embed_job(owner, &created)).await?;
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
        repositories::jobs::enqueue(&mut *tx, &embed_job(owner, &updated)).await?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn delete(&self, owner: &AccountId, id: i64) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        let deleted = repositories::footprints::delete(&mut *tx, id, owner).await?;
        if !deleted {
            return Err(AppError::NotFound(format!("Footprint #{id} not found")));
        }
        // Same transaction as the row delete, or search keeps returning a ghost.
        repositories::knowledge::delete_embeddings_for_item(
            &mut *tx,
            owner,
            KnowledgeItemType::Footprint.as_str(),
            id,
        )
        .await?;
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

/// Embedding job keyed by the footprint's version, so an edit is not swallowed
/// by the finished job of the version before it.
fn embed_job(owner: &AccountId, footprint: &Footprint) -> NewJob {
    NewJob::knowledge_embed_item_versioned(
        *owner,
        KnowledgeItemType::Footprint,
        footprint.id,
        &footprint.updated_at.to_rfc3339(),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate::run_migrations_unlocked, pool::seed_accounts, pool::test_pool};
    use crate::infrastructure::postgres::repositories::outbox::event_types_for;

    #[tokio::test]
    async fn footprint_service_validates_inputs_handles_currencies_and_publishes_events() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        seed_accounts(&pool, &["fp_svc_alice", "fp_svc_bob"]).await;
        let alice = AccountId::from_seed("fp_svc_alice");
        let bob = AccountId::from_seed("fp_svc_bob");

        let settings = SettingsService::new(pool.clone());
        let svc = FootprintService::new(pool.clone(), settings);

        // Validation: empty title, oversized title, oversized note, negative spend
        let base_input = CreateFootprintInput {
            title: "Kyoto".to_string(),
            latitude: 35.0116,
            longitude: 135.7681,
            country: Some("Japan".to_string()),
            city: Some("Kyoto".to_string()),
            address: None,
            arrival_date: "2026-05-01".to_string(),
            departure_date: Some("2026-05-04".to_string()),
            spend_amount: Some("0".to_string()),
            spend_currency: Some("JPY".to_string()),
            rating: Some(6), // clamps to 5
            category: None,
            note: Some("Shrines".to_string()),
        };

        assert!(matches!(
            svc.create(
                &alice,
                CreateFootprintInput {
                    title: "   ".to_string(),
                    ..base_input.clone()
                }
            )
            .await,
            Err(AppError::Validation(_))
        ));

        assert!(matches!(
            svc.create(
                &alice,
                CreateFootprintInput {
                    title: "x".repeat(MAX_TITLE_LEN + 1),
                    ..base_input.clone()
                }
            )
            .await,
            Err(AppError::Validation(_))
        ));

        assert!(matches!(
            svc.create(
                &alice,
                CreateFootprintInput {
                    note: Some("n".repeat(MAX_NOTE_LEN + 1)),
                    ..base_input.clone()
                }
            )
            .await,
            Err(AppError::Validation(_))
        ));

        assert!(matches!(
            svc.create(
                &alice,
                CreateFootprintInput {
                    spend_amount: Some("-10".to_string()),
                    ..base_input.clone()
                }
            )
            .await,
            Err(AppError::Validation(_))
        ));

        // Create with $0 spend in JPY (exponent 0)
        let fp = svc.create(&alice, base_input).await.expect("create fp");
        assert_eq!(fp.title, "Kyoto");
        assert_eq!(fp.stay_duration_days, 3);
        assert_eq!(fp.spend_amount_minor, 0);
        assert_eq!(fp.spend_currency, "JPY");
        assert_eq!(fp.spend_currency_exponent, 0);
        assert_eq!(fp.rating, 5);
        assert_eq!(fp.category, "travel");

        // Update spend to 1500 JPY, then switch currency to USD without supplying new spend_amount
        // so exponent conversion (0 -> 2) rescales 1500 -> 150000 minor units.
        let fp = svc
            .update(
                &alice,
                fp.id,
                UpdateFootprintInput {
                    spend_amount: Some("1500".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update spend");
        assert_eq!(fp.spend_amount_minor, 1500);
        assert_eq!(fp.spend_currency_exponent, 0);

        let fp = svc
            .update(
                &alice,
                fp.id,
                UpdateFootprintInput {
                    title: Some("Kyoto Station".to_string()),
                    spend_currency: Some("USD".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update currency exponent");
        assert_eq!(fp.title, "Kyoto Station");
        assert_eq!(fp.spend_currency, "USD");
        assert_eq!(fp.spend_currency_exponent, 2);
        assert_eq!(fp.spend_amount_minor, 150_000);

        // List & Summary stats
        let list = svc
            .list(&alice, Some("Japan"), Some("travel"), Some("Station"))
            .await
            .expect("list");
        assert_eq!(list.len(), 1);

        let stats = svc.summary_stats(&alice).await.expect("summary_stats");
        assert_eq!(stats.total_places, 1);
        assert_eq!(stats.total_countries, 1);
        assert_eq!(stats.total_days_traveled, 3);
        assert_eq!(stats.spend_by_currency.len(), 1);
        assert_eq!(stats.spend_by_currency[0].currency, "USD");
        assert_eq!(stats.spend_by_currency[0].total_spend_minor, 150_000);

        // Owner isolation: Bob cannot read, update, or delete Alice's footprint
        assert!(matches!(
            svc.get(&bob, fp.id).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            svc.delete(&bob, fp.id).await,
            Err(AppError::NotFound(_))
        ));

        // Delete & verify outbox events
        svc.delete(&alice, fp.id).await.expect("delete fp");
        let events = event_types_for(&pool, &alice).await.expect("events");
        assert_eq!(
            events,
            vec![
                "FootprintCreated",
                "FootprintUpdated",
                "FootprintUpdated",
                "FootprintDeleted"
            ]
        );
    }

    fn input(title: &str) -> CreateFootprintInput {
        CreateFootprintInput {
            title: title.to_string(),
            latitude: 35.0,
            longitude: 135.7,
            country: Some("Japan".to_string()),
            city: None,
            address: None,
            arrival_date: "2026-05-01".to_string(),
            departure_date: None,
            spend_amount: None,
            spend_currency: Some("USD".to_string()),
            rating: None,
            category: None,
            note: None,
        }
    }

    async fn embed_status(pool: &PgPool, fp: &Footprint) -> Option<crate::domain::jobs::JobStatus> {
        let key = format!("footprint:{}:{}", fp.id, fp.updated_at.to_rfc3339());
        repositories::jobs::status_by_key(pool, "knowledge.embed_item", &key)
            .await
            .unwrap()
    }

    /// Given a footprint, When it is created then edited, Then each version has
    /// its own embedding job.
    #[tokio::test]
    async fn create_and_update_enqueue_versioned_embed_jobs() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        seed_accounts(&pool, &["fp_embed_alice"]).await;
        let alice = AccountId::from_seed("fp_embed_alice");
        let svc = FootprintService::new(pool.clone(), SettingsService::new(pool.clone()));

        let created = svc.create(&alice, input("Kyoto")).await.unwrap();
        assert!(embed_status(&pool, &created).await.is_some());

        let updated = svc
            .update(
                &alice,
                created.id,
                UpdateFootprintInput {
                    title: Some("Kyoto Station".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_ne!(created.updated_at, updated.updated_at);
        assert!(embed_status(&pool, &updated).await.is_some());
    }

    /// Given an embedded footprint, When a stranger deletes it, Then the
    /// embeddings survive; When the owner deletes it, Then they are gone.
    #[tokio::test]
    async fn delete_removes_embeddings_only_for_the_owner() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        seed_accounts(&pool, &["fp_clean_alice", "fp_clean_bob"]).await;
        let alice = AccountId::from_seed("fp_clean_alice");
        let bob = AccountId::from_seed("fp_clean_bob");
        let svc = FootprintService::new(pool.clone(), SettingsService::new(pool.clone()));
        let fp = svc.create(&alice, input("Kyoto")).await.unwrap();
        repositories::knowledge::upsert_embedding_chunk(
            &pool,
            &alice,
            "footprint",
            fp.id,
            0,
            "snippet",
            &vec![0.0; 384],
            "model-a",
        )
        .await
        .unwrap();
        let chunks = |pool: PgPool| async move {
            repositories::knowledge::list_chunks(&pool, &alice, "footprint", fp.id)
                .await
                .unwrap()
                .len()
        };

        assert!(svc.delete(&bob, fp.id).await.is_err());
        assert_eq!(chunks(pool.clone()).await, 1);

        svc.delete(&alice, fp.id).await.unwrap();
        assert_eq!(chunks(pool.clone()).await, 0);
    }
}
