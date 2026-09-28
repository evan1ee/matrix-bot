//! PostgreSQL repository for footprints.
//!
//! Compile-checked queries over `impl sqlx::PgExecutor`. Maps private `FootprintRow`
//! into domain `Footprint` before leaving this file.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::footprints::{
    CreateFootprint, Footprint, FootprintCurrencySpend, FootprintsSummary, UpdateFootprint,
};

#[derive(Debug, sqlx::FromRow)]
struct FootprintRow {
    id: i64,
    owner_id: String,
    title: String,
    latitude: f64,
    longitude: f64,
    country: Option<String>,
    city: Option<String>,
    address: Option<String>,
    arrival_date: String,
    departure_date: Option<String>,
    stay_duration_days: i64,
    spend_amount_minor: i64,
    spend_currency: String,
    spend_currency_exponent: i64,
    rating: i64,
    category: String,
    note: Option<String>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

impl From<FootprintRow> for Footprint {
    fn from(row: FootprintRow) -> Self {
        Self {
            id: row.id,
            owner_id: AccountId::new(row.owner_id),
            title: row.title,
            latitude: row.latitude,
            longitude: row.longitude,
            country: row.country,
            city: row.city,
            address: row.address,
            arrival_date: row.arrival_date,
            departure_date: row.departure_date,
            stay_duration_days: row.stay_duration_days,
            spend_amount_minor: row.spend_amount_minor,
            spend_currency: row.spend_currency,
            spend_currency_exponent: row.spend_currency_exponent,
            rating: row.rating,
            category: row.category,
            note: row.note,
            created_at: row.created_at.unwrap_or_else(Utc::now),
            updated_at: row.updated_at.unwrap_or_else(Utc::now),
        }
    }
}

pub async fn create<'e>(
    conn: impl PgExecutor<'e>,
    owner: &AccountId,
    f: &CreateFootprint,
) -> Result<Footprint, sqlx::Error> {
    let row = sqlx::query_as!(
        FootprintRow,
        r#"
        INSERT INTO footprints (
            owner_id, title, latitude, longitude, country, city, address,
            arrival_date, departure_date, stay_duration_days,
            spend_amount_minor, spend_currency, spend_currency_exponent,
            rating, category, note
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
        RETURNING id, owner_id, title, latitude, longitude, country, city, address,
                  arrival_date, departure_date, stay_duration_days,
                  spend_amount_minor, spend_currency, spend_currency_exponent,
                  rating, category, note, created_at, updated_at
        "#,
        owner.as_str(),
        f.title,
        f.latitude,
        f.longitude,
        f.country.as_deref(),
        f.city.as_deref(),
        f.address.as_deref(),
        f.arrival_date,
        f.departure_date.as_deref(),
        f.stay_duration_days,
        f.spend_amount_minor,
        f.spend_currency,
        f.spend_currency_exponent,
        f.rating,
        f.category,
        f.note.as_deref()
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn find<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
) -> Result<Option<Footprint>, sqlx::Error> {
    let row = sqlx::query_as!(
        FootprintRow,
        r#"
        SELECT id, owner_id, title, latitude, longitude, country, city, address,
               arrival_date, departure_date, stay_duration_days,
               spend_amount_minor, spend_currency, spend_currency_exponent,
               rating, category, note, created_at, updated_at
        FROM footprints
        WHERE id = $1
        "#,
        id
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_owned<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    owner: &AccountId,
) -> Result<Option<Footprint>, sqlx::Error> {
    let row = sqlx::query_as!(
        FootprintRow,
        r#"
        SELECT id, owner_id, title, latitude, longitude, country, city, address,
               arrival_date, departure_date, stay_duration_days,
               spend_amount_minor, spend_currency, spend_currency_exponent,
               rating, category, note, created_at, updated_at
        FROM footprints
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_str()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_by_owner<'e>(
    conn: impl PgExecutor<'e>,
    owner: &AccountId,
    country: Option<&str>,
    category: Option<&str>,
    search: Option<&str>,
) -> Result<Vec<Footprint>, sqlx::Error> {
    let search_pattern = search.map(|s| format!("%{}%", s));
    let rows = sqlx::query_as!(
        FootprintRow,
        r#"
        SELECT id, owner_id, title, latitude, longitude, country, city, address,
               arrival_date, departure_date, stay_duration_days,
               spend_amount_minor, spend_currency, spend_currency_exponent,
               rating, category, note, created_at, updated_at
        FROM footprints
        WHERE owner_id = $1
          AND ($2::text IS NULL OR country = $2)
          AND ($3::text IS NULL OR category = $3)
          AND ($4::text IS NULL OR title LIKE $4 OR city LIKE $4 OR address LIKE $4 OR note LIKE $4)
        ORDER BY arrival_date DESC, id DESC
        "#,
        owner.as_str(),
        country,
        category,
        search_pattern.as_deref()
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn update<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    owner: &AccountId,
    f: &UpdateFootprint,
) -> Result<Option<Footprint>, sqlx::Error> {
    let row = sqlx::query_as!(
        FootprintRow,
        r#"
        UPDATE footprints
        SET title = $1, latitude = $2, longitude = $3, country = $4, city = $5, address = $6,
            arrival_date = $7, departure_date = $8, stay_duration_days = $9,
            spend_amount_minor = $10, spend_currency = $11, spend_currency_exponent = $12,
            rating = $13, category = $14, note = $15, updated_at = CURRENT_TIMESTAMP
        WHERE id = $16 AND owner_id = $17
        RETURNING id, owner_id, title, latitude, longitude, country, city, address,
                  arrival_date, departure_date, stay_duration_days,
                  spend_amount_minor, spend_currency, spend_currency_exponent,
                  rating, category, note, created_at, updated_at
        "#,
        f.title,
        f.latitude,
        f.longitude,
        f.country.as_deref(),
        f.city.as_deref(),
        f.address.as_deref(),
        f.arrival_date,
        f.departure_date.as_deref(),
        f.stay_duration_days,
        f.spend_amount_minor,
        f.spend_currency,
        f.spend_currency_exponent,
        f.rating,
        f.category,
        f.note.as_deref(),
        id,
        owner.as_str()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn delete<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    owner: &AccountId,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        "DELETE FROM footprints WHERE id = $1 AND owner_id = $2",
        id,
        owner.as_str()
    )
    .execute(conn)
    .await?;

    Ok(res.rows_affected() > 0)
}

pub async fn summary_stats(
    pool: &sqlx::PgPool,
    owner: &AccountId,
) -> Result<FootprintsSummary, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT
            COUNT(id) AS "total_places!",
            COUNT(DISTINCT NULLIF(country, '')) AS "total_countries!",
            COALESCE(CAST(SUM(stay_duration_days) AS BIGINT), 0) AS "total_days_traveled!"
        FROM footprints
        WHERE owner_id = $1
        "#,
        owner.as_str()
    )
    .fetch_one(pool)
    .await?;

    let spends = sqlx::query!(
        r#"
        SELECT
            spend_currency,
            spend_currency_exponent,
            COALESCE(CAST(SUM(spend_amount_minor) AS BIGINT), 0) AS "total_spend_minor!"
        FROM footprints
        WHERE owner_id = $1 AND spend_amount_minor > 0
        GROUP BY spend_currency, spend_currency_exponent
        ORDER BY 3 DESC
        "#,
        owner.as_str()
    )
    .fetch_all(pool)
    .await?;

    let spend_by_currency = spends
        .into_iter()
        .map(|s| FootprintCurrencySpend {
            currency: s.spend_currency,
            currency_exponent: s.spend_currency_exponent,
            total_spend_minor: s.total_spend_minor,
        })
        .collect();

    Ok(FootprintsSummary {
        total_places: row.total_places,
        total_countries: row.total_countries,
        total_days_traveled: row.total_days_traveled,
        spend_by_currency,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_footprints_repository_crud() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let owner = AccountId::new("user_fp_test");
        let new_fp = CreateFootprint {
            title: "Tokyo Tower".to_string(),
            latitude: 35.6586,
            longitude: 139.7454,
            country: Some("Japan".to_string()),
            city: Some("Tokyo".to_string()),
            address: Some("Minato City".to_string()),
            arrival_date: "2026-04-01".to_string(),
            departure_date: Some("2026-04-05".to_string()),
            stay_duration_days: 4,
            spend_amount_minor: 12000,
            spend_currency: "JPY".to_string(),
            spend_currency_exponent: 0,
            rating: 5,
            category: "sightseeing".to_string(),
            note: Some("Great view at night".to_string()),
        };

        let created = create(&pool, &owner, &new_fp).await.unwrap();
        assert_eq!(created.title, "Tokyo Tower");
        assert_eq!(created.stay_duration_days, 4);
        assert_eq!(created.rating, 5);

        let found = find_owned(&pool, created.id, &owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.title, "Tokyo Tower");

        let update_fp = UpdateFootprint {
            title: "Tokyo Tower Main Deck".to_string(),
            latitude: 35.6586,
            longitude: 139.7454,
            country: Some("Japan".to_string()),
            city: Some("Tokyo".to_string()),
            address: Some("Minato City".to_string()),
            arrival_date: "2026-04-01".to_string(),
            departure_date: Some("2026-04-06".to_string()),
            stay_duration_days: 5,
            spend_amount_minor: 15000,
            spend_currency: "JPY".to_string(),
            spend_currency_exponent: 0,
            rating: 5,
            category: "sightseeing".to_string(),
            note: Some("Updated note".to_string()),
        };
        let updated = update(&pool, created.id, &owner, &update_fp)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.title, "Tokyo Tower Main Deck");
        assert_eq!(updated.stay_duration_days, 5);

        let list = list_by_owner(&pool, &owner, Some("Japan"), None, Some("Tower"))
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, created.id);

        let stats = summary_stats(&pool, &owner).await.unwrap();
        assert_eq!(stats.total_places, 1);
        assert_eq!(stats.total_countries, 1);
        assert_eq!(stats.total_days_traveled, 5);
        assert_eq!(stats.spend_by_currency.len(), 1);
        assert_eq!(stats.spend_by_currency[0].total_spend_minor, 15000);

        let deleted = delete(&pool, created.id, &owner).await.unwrap();
        assert!(deleted);

        let missing = find_owned(&pool, created.id, &owner).await.unwrap();
        assert!(missing.is_none());
    }
}
