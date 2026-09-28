use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::domain::accounts::AccountId;
use crate::domain::finance::Category;

#[derive(Debug, sqlx::FromRow)]
struct CategoryRow {
    id: i64,
    owner_id: String,
    name: String,
    created_at: Option<DateTime<Utc>>,
}

impl From<CategoryRow> for Category {
    fn from(r: CategoryRow) -> Self {
        Self {
            id: r.id,
            owner_id: AccountId::new(r.owner_id),
            name: r.name,
            created_at: r.created_at,
        }
    }
}

pub async fn find_or_create_category(
    pool: &PgPool,
    owner_id: &AccountId,
    name: &str,
) -> Result<Option<Category>, sqlx::Error> {
    if let Some(cat) = find_category_by_name(pool, owner_id, name).await? {
        return Ok(Some(cat));
    }
    create_category(pool, owner_id, name).await
}

pub async fn create_category(
    pool: &PgPool,
    owner_id: &AccountId,
    name: &str,
) -> Result<Option<Category>, sqlx::Error> {
    let row = sqlx::query_as!(
        CategoryRow,
        r#"
        INSERT INTO finance_categories (owner_id, name)
        VALUES ($1, $2)
        ON CONFLICT (owner_id, name) DO NOTHING
        RETURNING id, owner_id, name, created_at
        "#,
        owner_id.as_str(),
        name
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_categories(
    pool: &PgPool,
    owner_id: &AccountId,
) -> Result<Vec<Category>, sqlx::Error> {
    let rows = sqlx::query_as!(
        CategoryRow,
        r#"
        SELECT id, owner_id, name, created_at
        FROM finance_categories
        WHERE owner_id = $1
        ORDER BY LOWER(name) ASC
        "#,
        owner_id.as_str()
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn find_category_by_name(
    pool: &PgPool,
    owner_id: &AccountId,
    name: &str,
) -> Result<Option<Category>, sqlx::Error> {
    let row = sqlx::query_as!(
        CategoryRow,
        r#"
        SELECT id, owner_id, name, created_at
        FROM finance_categories
        WHERE owner_id = $1 AND LOWER(name) = LOWER($2)
        "#,
        owner_id.as_str(),
        name
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_category_by_id(
    pool: &PgPool,
    owner_id: &AccountId,
    id: i64,
) -> Result<Option<Category>, sqlx::Error> {
    let row = sqlx::query_as!(
        CategoryRow,
        r#"
        SELECT id, owner_id, name, created_at
        FROM finance_categories
        WHERE owner_id = $1 AND id = $2
        "#,
        owner_id.as_str(),
        id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn delete_category(
    pool: &PgPool,
    owner_id: &AccountId,
    id: i64,
) -> Result<Option<u64>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let owns = sqlx::query_scalar!(
        "SELECT id FROM finance_categories WHERE id = $1 AND owner_id = $2 FOR UPDATE",
        id,
        owner_id.as_str()
    )
    .fetch_optional(&mut *tx)
    .await?;

    if owns.is_none() {
        return Ok(None);
    }

    let orphaned = sqlx::query!(
        "UPDATE transactions SET category_id = NULL WHERE category_id = $1 AND owner_id = $2",
        id,
        owner_id.as_str()
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();

    sqlx::query!(
        "DELETE FROM finance_categories WHERE id = $1 AND owner_id = $2",
        id,
        owner_id.as_str()
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Some(orphaned))
}
