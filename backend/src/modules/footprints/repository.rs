use crate::db::DbPool;
use anyhow::Result;

use super::models::{
    calculate_stay_duration, Footprint, FootprintCurrencySpend, FootprintsSummary,
};

pub struct FootprintRepository;

impl FootprintRepository {
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        pool: &DbPool,
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

        let footprint = crate::db_fetch_one!(
            pool,
            Footprint,
            r#"
            INSERT INTO footprints (
                owner_id, title, latitude, longitude, country, city, address,
                arrival_date, departure_date, stay_duration_days,
                spend_amount_minor, spend_currency, spend_currency_exponent,
                rating, category, note
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id, owner_id, title, latitude, longitude, country, city, address,
                      arrival_date, departure_date, stay_duration_days,
                      spend_amount_minor, spend_currency, spend_currency_exponent,
                      rating, category, note, created_at, updated_at;
            "#,
            owner_id,
            title,
            latitude,
            longitude,
            country,
            city,
            address,
            arrival_date,
            departure_date,
            duration,
            spend_amount_minor,
            spend_currency,
            spend_currency_exponent,
            rating,
            category,
            note
        )?;

        Ok(footprint)
    }

    pub async fn find(pool: &DbPool, id: i64) -> Result<Option<Footprint>> {
        let fp = crate::db_fetch_optional!(
            pool,
            Footprint,
            "SELECT * FROM footprints WHERE id = ?;",
            id
        )?;

        Ok(fp)
    }

    pub async fn find_owned(pool: &DbPool, id: i64, owner_id: &str) -> Result<Option<Footprint>> {
        let fp = crate::db_fetch_optional!(
            pool,
            Footprint,
            "SELECT * FROM footprints WHERE id = ? AND owner_id = ?;",
            id,
            owner_id
        )?;

        Ok(fp)
    }

    pub async fn list_by_owner(
        pool: &DbPool,
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
            query.push_str(" AND (title LIKE ? OR city LIKE ? OR address LIKE ? OR note LIKE ?)");
        }
        query.push_str(" ORDER BY arrival_date DESC, id DESC;");

        // Bind values are collected in the same order the clauses were appended
        // above, then handed to the macro as one runtime-sized list.
        let mut binds: Vec<String> = vec![owner_id.to_string()];
        if let Some(c) = country {
            binds.push(c.to_string());
        }
        if let Some(cat) = category {
            binds.push(cat.to_string());
        }
        if let Some(s) = search {
            let pattern = format!("%{}%", s);
            for _ in 0..4 {
                binds.push(pattern.clone());
            }
        }

        let items = crate::db_fetch_all_dyn!(pool, Footprint, &query, binds)?;
        Ok(items)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        pool: &DbPool,
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

        let updated = crate::db_fetch_optional!(
            pool,
            Footprint,
            r#"
            UPDATE footprints
            SET title = ?, latitude = ?, longitude = ?, country = ?, city = ?, address = ?,
                arrival_date = ?, departure_date = ?, stay_duration_days = ?,
                spend_amount_minor = ?, spend_currency = ?, spend_currency_exponent = ?,
                rating = ?, category = ?, note = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ? AND owner_id = ?
            RETURNING id, owner_id, title, latitude, longitude, country, city, address,
                      arrival_date, departure_date, stay_duration_days,
                      spend_amount_minor, spend_currency, spend_currency_exponent,
                      rating, category, note, created_at, updated_at;
            "#,
            title,
            latitude,
            longitude,
            country,
            city,
            address,
            arrival_date,
            departure_date,
            duration,
            spend_amount_minor,
            spend_currency,
            spend_currency_exponent,
            rating,
            category,
            note,
            id,
            owner_id
        )?;

        Ok(updated)
    }

    pub async fn delete(pool: &DbPool, id: i64, owner_id: &str) -> Result<bool> {
        let rows = crate::db_execute!(
            pool,
            "DELETE FROM footprints WHERE id = ? AND owner_id = ?;",
            id,
            owner_id
        )?;

        Ok(rows > 0)
    }

    pub async fn summary_stats(pool: &DbPool, owner_id: &str) -> Result<FootprintsSummary> {
        let row = crate::db_fetch_one!(
            pool,
            (i64, i64, i64),
            r#"
            SELECT
                COUNT(id) AS total_places,
                COUNT(DISTINCT NULLIF(country, '')) AS total_countries,
                COALESCE(CAST(SUM(stay_duration_days) AS BIGINT), 0) AS total_days_traveled
            FROM footprints
            WHERE owner_id = ?;
            "#,
            owner_id
        )?;

        let spends = crate::db_fetch_all!(
            pool,
            (String, i64, i64),
            r#"
            SELECT spend_currency, spend_currency_exponent, CAST(SUM(spend_amount_minor) AS BIGINT)
            FROM footprints
            WHERE owner_id = ? AND spend_amount_minor > 0
            GROUP BY spend_currency, spend_currency_exponent
            ORDER BY CAST(SUM(spend_amount_minor) AS BIGINT) DESC;
            "#,
            owner_id
        )?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[tokio::test]
    async fn test_create_and_update_atomic_returning() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::schema::run_migrations(&pool).await.unwrap();
        let db = Database { pool };

        let created = FootprintRepository::create(
            &db.pool,
            "user_test",
            "Initial Place",
            30.0,
            120.0,
            Some("Country A"),
            Some("City A"),
            Some("Address A"),
            "2026-09-01",
            Some("2026-09-03"),
            1000,
            "USD",
            2,
            4,
            "travel",
            Some("Note 1"),
        )
        .await
        .expect("create must succeed and return Footprint");

        assert_eq!(created.title, "Initial Place");
        assert_eq!(created.stay_duration_days, 2);
        assert_eq!(created.rating, 4);

        let updated = FootprintRepository::update(
            &db.pool,
            created.id,
            "user_test",
            "Updated Place",
            30.5,
            120.5,
            Some("Country B"),
            Some("City B"),
            Some("Address B"),
            "2026-09-01",
            Some("2026-09-05"),
            2500,
            "USD",
            2,
            5,
            "food",
            Some("Note 2"),
        )
        .await
        .expect("update query must succeed")
        .expect("updated footprint must exist");

        assert_eq!(updated.title, "Updated Place");
        assert_eq!(updated.stay_duration_days, 4);
        assert_eq!(updated.rating, 5);
        assert_eq!(updated.category, "food");

        let found = FootprintRepository::find(&db.pool, created.id)
            .await
            .unwrap()
            .expect("find must return updated footprint");
        assert_eq!(found.title, "Updated Place");
    }
}
