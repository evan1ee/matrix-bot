use anyhow::Result;
use sqlx::SqlitePool;

use super::models::{calculate_stay_duration, Footprint, FootprintCurrencySpend, FootprintsSummary};

pub struct FootprintRepository;

impl FootprintRepository {
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        pool: &SqlitePool,
        owner_id: &str,
        title: &str,
        latitude: f64,
        longitude: f64,
        country: Option<&str>,
        city: Option<&str>,
        address: Option<&str>,
        arrival_date: &str,
        departure_date: Option<&str>,
        spend_amount_minor: i64,
        spend_currency: &str,
        spend_currency_exponent: i64,
        rating: i64,
        category: &str,
        note: Option<&str>,
    ) -> Result<Footprint> {
        let duration = calculate_stay_duration(arrival_date, departure_date);

        let id = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO footprints (
                owner_id, title, latitude, longitude, country, city, address,
                arrival_date, departure_date, stay_duration_days,
                spend_amount_minor, spend_currency, spend_currency_exponent,
                rating, category, note
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id;
            "#,
        )
        .bind(owner_id)
        .bind(title)
        .bind(latitude)
        .bind(longitude)
        .bind(country)
        .bind(city)
        .bind(address)
        .bind(arrival_date)
        .bind(departure_date)
        .bind(duration)
        .bind(spend_amount_minor)
        .bind(spend_currency)
        .bind(spend_currency_exponent)
        .bind(rating)
        .bind(category)
        .bind(note)
        .fetch_one(pool)
        .await?;

        let footprint = Self::find(pool, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Footprint missing immediately after insert"))?;

        Ok(footprint)
    }

    pub async fn find(pool: &SqlitePool, id: i64) -> Result<Option<Footprint>> {
        let fp = sqlx::query_as::<_, Footprint>(
            "SELECT * FROM footprints WHERE id = ?;",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(fp)
    }

    pub async fn find_owned(pool: &SqlitePool, id: i64, owner_id: &str) -> Result<Option<Footprint>> {
        let fp = sqlx::query_as::<_, Footprint>(
            "SELECT * FROM footprints WHERE id = ? AND owner_id = ?;",
        )
        .bind(id)
        .bind(owner_id)
        .fetch_optional(pool)
        .await?;

        Ok(fp)
    }

    pub async fn list_by_owner(
        pool: &SqlitePool,
        owner_id: &str,
        country: Option<&str>,
        category: Option<&str>,
        search: Option<&str>,
    ) -> Result<Vec<Footprint>> {
        let mut query = String::from("SELECT * FROM footprints WHERE owner_id = ?");
        if country.is_some() {
            query.push_str(" AND country = ?");
        }
        if category.is_some() {
            query.push_str(" AND category = ?");
        }
        if search.is_some() {
            query.push_str(" AND (title LIKE ? OR city LIKE ? OR country LIKE ? OR note LIKE ?)");
        }
        query.push_str(" ORDER BY arrival_date DESC, id DESC;");

        let mut q = sqlx::query_as::<_, Footprint>(&query).bind(owner_id);
        if let Some(c) = country {
            q = q.bind(c);
        }
        if let Some(cat) = category {
            q = q.bind(cat);
        }
        if let Some(s) = search {
            let pat = format!("%{}%", s);
            q = q.bind(pat.clone()).bind(pat.clone()).bind(pat.clone()).bind(pat);
        }

        let list = q.fetch_all(pool).await?;
        Ok(list)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        pool: &SqlitePool,
        id: i64,
        owner_id: &str,
        title: &str,
        latitude: f64,
        longitude: f64,
        country: Option<&str>,
        city: Option<&str>,
        address: Option<&str>,
        arrival_date: &str,
        departure_date: Option<&str>,
        spend_amount_minor: i64,
        spend_currency: &str,
        spend_currency_exponent: i64,
        rating: i64,
        category: &str,
        note: Option<&str>,
    ) -> Result<Option<Footprint>> {
        let duration = calculate_stay_duration(arrival_date, departure_date);

        let rows = sqlx::query(
            r#"
            UPDATE footprints
            SET title = ?, latitude = ?, longitude = ?, country = ?, city = ?, address = ?,
                arrival_date = ?, departure_date = ?, stay_duration_days = ?,
                spend_amount_minor = ?, spend_currency = ?, spend_currency_exponent = ?,
                rating = ?, category = ?, note = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ? AND owner_id = ?;
            "#,
        )
        .bind(title)
        .bind(latitude)
        .bind(longitude)
        .bind(country)
        .bind(city)
        .bind(address)
        .bind(arrival_date)
        .bind(departure_date)
        .bind(duration)
        .bind(spend_amount_minor)
        .bind(spend_currency)
        .bind(spend_currency_exponent)
        .bind(rating)
        .bind(category)
        .bind(note)
        .bind(id)
        .bind(owner_id)
        .execute(pool)
        .await?
        .rows_affected();

        if rows == 0 {
            return Ok(None);
        }

        Self::find_owned(pool, id, owner_id).await
    }

    pub async fn delete(pool: &SqlitePool, id: i64, owner_id: &str) -> Result<bool> {
        let rows = sqlx::query("DELETE FROM footprints WHERE id = ? AND owner_id = ?;")
            .bind(id)
            .bind(owner_id)
            .execute(pool)
            .await?
            .rows_affected();

        Ok(rows > 0)
    }

    pub async fn summary_stats(pool: &SqlitePool, owner_id: &str) -> Result<FootprintsSummary> {
        let row = sqlx::query_as::<_, (i64, i64, i64)>(
            r#"
            SELECT
                COUNT(id) AS total_places,
                COUNT(DISTINCT NULLIF(country, '')) AS total_countries,
                COALESCE(SUM(stay_duration_days), 0) AS total_days_traveled
            FROM footprints
            WHERE owner_id = ?;
            "#,
        )
        .bind(owner_id)
        .fetch_one(pool)
        .await?;

        let spends = sqlx::query_as::<_, (String, i64, i64)>(
            r#"
            SELECT spend_currency, spend_currency_exponent, SUM(spend_amount_minor)
            FROM footprints
            WHERE owner_id = ? AND spend_amount_minor > 0
            GROUP BY spend_currency, spend_currency_exponent
            ORDER BY SUM(spend_amount_minor) DESC;
            "#,
        )
        .bind(owner_id)
        .fetch_all(pool)
        .await?;

        let spend_by_currency = spends
            .into_iter()
            .map(|(cur, exp, total)| FootprintCurrencySpend {
                currency: cur,
                currency_exponent: exp,
                total_spend_minor: total,
            })
            .collect();

        Ok(FootprintsSummary {
            total_places: row.0,
            total_countries: row.1,
            total_days_traveled: row.2,
            spend_by_currency,
        })
    }
}
