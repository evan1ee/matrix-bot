use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};

use crate::domain::accounts::AccountId;
use crate::domain::portfolio::audit::{
    compute_entry_hash, AuditVerificationResult, PortfolioAuditEntry, GENESIS_HASH,
};

#[allow(clippy::too_many_arguments)]
pub async fn record_audit_entry_tx(
    conn: &mut PgConnection,
    owner_id: &AccountId,
    action: &str,
    symbol: Option<&str>,
    quantity_minor: Option<i64>,
    price_minor: Option<i64>,
    amount_minor: i64,
    currency: &str,
    currency_exponent: i64,
    realized_pl_minor: Option<i64>,
    note: Option<&str>,
    custom_timestamp: Option<DateTime<Utc>>,
) -> Result<PortfolioAuditEntry, sqlx::Error> {
    let now = custom_timestamp.unwrap_or_else(Utc::now);
    let epoch_ms = now.timestamp_millis();

    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(owner_id.to_string())
        .execute(&mut *conn)
        .await?;

    let prev_row = sqlx::query!(
        r#"
        SELECT entry_hash
        FROM portfolio_audit_log
        WHERE owner_id = $1
        ORDER BY id DESC
        LIMIT 1
        "#,
        owner_id.as_uuid()
    )
    .fetch_optional(&mut *conn)
    .await?;

    let prev_hash = prev_row
        .map(|r| r.entry_hash)
        .unwrap_or_else(|| GENESIS_HASH.to_string());

    let entry_hash = compute_entry_hash(
        &prev_hash,
        &owner_id.to_string(),
        action,
        symbol,
        quantity_minor,
        price_minor,
        amount_minor,
        currency,
        currency_exponent,
        realized_pl_minor,
        note,
        epoch_ms,
    );

    let row = sqlx::query!(
        r#"
        INSERT INTO portfolio_audit_log
         (owner_id, action, symbol, quantity_minor, price_minor, amount_minor, currency,
          currency_exponent, realized_pl_minor, note, prev_hash, entry_hash, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        RETURNING id, owner_id, action, symbol, quantity_minor, price_minor,
                  amount_minor, currency, currency_exponent, realized_pl_minor,
                  note, prev_hash, entry_hash, created_at
        "#,
        owner_id.as_uuid(),
        action,
        symbol,
        quantity_minor,
        price_minor,
        amount_minor,
        currency,
        currency_exponent,
        realized_pl_minor,
        note,
        prev_hash,
        entry_hash,
        now
    )
    .fetch_one(&mut *conn)
    .await?;

    Ok(PortfolioAuditEntry {
        id: row.id,
        owner_id: AccountId::from_uuid(row.owner_id),
        action: row.action,
        symbol: row.symbol,
        quantity_minor: row.quantity_minor,
        price_minor: row.price_minor,
        amount_minor: row.amount_minor,
        currency: row.currency,
        currency_exponent: row.currency_exponent,
        realized_pl_minor: row.realized_pl_minor,
        note: row.note,
        prev_hash: row.prev_hash,
        entry_hash: row.entry_hash,
        created_at: row.created_at,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn record_audit_entry(
    pool: &PgPool,
    owner_id: &AccountId,
    action: &str,
    symbol: Option<&str>,
    quantity_minor: Option<i64>,
    price_minor: Option<i64>,
    amount_minor: i64,
    currency: &str,
    currency_exponent: i64,
    realized_pl_minor: Option<i64>,
    note: Option<&str>,
    custom_timestamp: Option<DateTime<Utc>>,
) -> Result<PortfolioAuditEntry, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let entry = record_audit_entry_tx(
        &mut tx,
        owner_id,
        action,
        symbol,
        quantity_minor,
        price_minor,
        amount_minor,
        currency,
        currency_exponent,
        realized_pl_minor,
        note,
        custom_timestamp,
    )
    .await?;
    tx.commit().await?;
    Ok(entry)
}

pub async fn list_audit_entries(
    pool: &PgPool,
    owner_id: &AccountId,
    limit: i64,
    offset: i64,
) -> Result<Vec<PortfolioAuditEntry>, sqlx::Error> {
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);

    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, action, symbol, quantity_minor, price_minor,
               amount_minor, currency, currency_exponent, realized_pl_minor,
               note, prev_hash, entry_hash, created_at
        FROM portfolio_audit_log
        WHERE owner_id = $1
        ORDER BY id DESC
        LIMIT $2 OFFSET $3
        "#,
        owner_id.as_uuid(),
        limit,
        offset
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| PortfolioAuditEntry {
            id: row.id,
            owner_id: AccountId::from_uuid(row.owner_id),
            action: row.action,
            symbol: row.symbol,
            quantity_minor: row.quantity_minor,
            price_minor: row.price_minor,
            amount_minor: row.amount_minor,
            currency: row.currency,
            currency_exponent: row.currency_exponent,
            realized_pl_minor: row.realized_pl_minor,
            note: row.note,
            prev_hash: row.prev_hash,
            entry_hash: row.entry_hash,
            created_at: row.created_at,
        })
        .collect())
}

pub async fn verify_audit_chain(
    pool: &PgPool,
    owner_id: &AccountId,
) -> Result<AuditVerificationResult, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, action, symbol, quantity_minor, price_minor,
               amount_minor, currency, currency_exponent, realized_pl_minor,
               note, prev_hash, entry_hash, created_at
        FROM portfolio_audit_log
        WHERE owner_id = $1
        ORDER BY id ASC
        "#,
        owner_id.as_uuid()
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(AuditVerificationResult {
            is_valid: true,
            total_verified: 0,
            genesis_hash: GENESIS_HASH.to_string(),
            head_hash: None,
            broken_entry_id: None,
            message: "Audit ledger is empty (no operations recorded yet)".to_string(),
        });
    }

    let entries: Vec<PortfolioAuditEntry> = rows
        .into_iter()
        .map(|row| PortfolioAuditEntry {
            id: row.id,
            owner_id: AccountId::from_uuid(row.owner_id),
            action: row.action,
            symbol: row.symbol,
            quantity_minor: row.quantity_minor,
            price_minor: row.price_minor,
            amount_minor: row.amount_minor,
            currency: row.currency,
            currency_exponent: row.currency_exponent,
            realized_pl_minor: row.realized_pl_minor,
            note: row.note,
            prev_hash: row.prev_hash,
            entry_hash: row.entry_hash,
            created_at: row.created_at,
        })
        .collect();

    let mut expected_prev_hash = GENESIS_HASH.to_string();

    for entry in &entries {
        if entry.prev_hash != expected_prev_hash {
            return Ok(AuditVerificationResult {
                is_valid: false,
                total_verified: entries.len(),
                genesis_hash: GENESIS_HASH.to_string(),
                head_hash: None,
                broken_entry_id: Some(entry.id),
                message: format!(
                    "Chain broken at entry #{}: prev_hash does not match previous entry's hash",
                    entry.id
                ),
            });
        }

        let epoch_ms = entry.created_at.map(|t| t.timestamp_millis()).unwrap_or(0);
        let computed = compute_entry_hash(
            &entry.prev_hash,
            &entry.owner_id.to_string(),
            &entry.action,
            entry.symbol.as_deref(),
            entry.quantity_minor,
            entry.price_minor,
            entry.amount_minor,
            &entry.currency,
            entry.currency_exponent,
            entry.realized_pl_minor,
            entry.note.as_deref(),
            epoch_ms,
        );

        if computed != entry.entry_hash {
            return Ok(AuditVerificationResult {
                is_valid: false,
                total_verified: entries.len(),
                genesis_hash: GENESIS_HASH.to_string(),
                head_hash: None,
                broken_entry_id: Some(entry.id),
                message: format!(
                    "Data alteration detected at entry #{}: cryptographic signature mismatch",
                    entry.id
                ),
            });
        }

        expected_prev_hash = entry.entry_hash.clone();
    }

    Ok(AuditVerificationResult {
        is_valid: true,
        total_verified: entries.len(),
        genesis_hash: GENESIS_HASH.to_string(),
        head_hash: Some(expected_prev_hash),
        broken_entry_id: None,
        message: format!(
            "Cryptographic audit chain fully verified ({} entries intact)",
            entries.len()
        ),
    })
}

pub async fn absorb_ledger(
    conn: &mut PgConnection,
    from: &AccountId,
    into: &AccountId,
) -> Result<i64, sqlx::Error> {
    let moved: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = $1",
        from.as_uuid()
    )
    .fetch_one(&mut *conn)
    .await?
    .unwrap_or(0);

    if moved == 0 {
        return Ok(0);
    }

    const DROP_AUDIT_UPDATE_TRIGGER: &str =
        "DROP TRIGGER IF EXISTS prevent_portfolio_audit_update ON portfolio_audit_log;";
    sqlx::query(DROP_AUDIT_UPDATE_TRIGGER)
        .execute(&mut *conn)
        .await?;

    sqlx::query!(
        "UPDATE portfolio_audit_log SET owner_id = $1 WHERE owner_id = $2",
        into.as_uuid(),
        from.as_uuid()
    )
    .execute(&mut *conn)
    .await?;

    let note_text = format!("Absorbed {} ledger entries from account {}", moved, from);
    let now = Utc::now();
    sqlx::query!(
        r#"
        INSERT INTO portfolio_audit_log
         (owner_id, action, symbol, quantity_minor, price_minor, amount_minor, currency,
          currency_exponent, realized_pl_minor, note, prev_hash, entry_hash, created_at)
        VALUES ($1, 'merge', NULL, NULL, NULL, 0, '', 2, NULL, $2, $3, '', $4)
        "#,
        into.as_uuid(),
        note_text,
        GENESIS_HASH,
        now
    )
    .execute(&mut *conn)
    .await?;

    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, action, symbol, quantity_minor, price_minor,
               amount_minor, currency, currency_exponent, realized_pl_minor,
               note, prev_hash, entry_hash, created_at
        FROM portfolio_audit_log
        WHERE owner_id = $1
        ORDER BY id ASC
        "#,
        into.as_uuid()
    )
    .fetch_all(&mut *conn)
    .await?;

    let into_str = into.to_string();
    let mut prev = GENESIS_HASH.to_string();
    for row in rows {
        let epoch_ms = row.created_at.map(|t| t.timestamp_millis()).unwrap_or(0);
        let entry_hash = compute_entry_hash(
            &prev,
            &into_str,
            &row.action,
            row.symbol.as_deref(),
            row.quantity_minor,
            row.price_minor,
            row.amount_minor,
            &row.currency,
            row.currency_exponent,
            row.realized_pl_minor,
            row.note.as_deref(),
            epoch_ms,
        );

        sqlx::query!(
            "UPDATE portfolio_audit_log SET prev_hash = $1, entry_hash = $2 WHERE id = $3",
            prev,
            entry_hash,
            row.id
        )
        .execute(&mut *conn)
        .await?;

        prev = entry_hash;
    }

    const RESTORE_AUDIT_UPDATE_TRIGGER: [&str; 3] = [
        "CREATE OR REPLACE FUNCTION prevent_portfolio_audit_update() RETURNS trigger AS $fn$ \
         BEGIN RAISE EXCEPTION 'Audit log entries are immutable and cannot be updated'; END; \
         $fn$ LANGUAGE plpgsql;",
        "DROP TRIGGER IF EXISTS prevent_portfolio_audit_update ON portfolio_audit_log;",
        "CREATE TRIGGER prevent_portfolio_audit_update BEFORE UPDATE ON portfolio_audit_log \
         FOR EACH ROW EXECUTE FUNCTION prevent_portfolio_audit_update();",
    ];
    for statement in RESTORE_AUDIT_UPDATE_TRIGGER {
        sqlx::query(statement).execute(&mut *conn).await?;
    }

    Ok(moved)
}

pub async fn first_audit_entry_for_currency(
    pool: &PgPool,
    owner_id: &AccountId,
    currency: &str,
) -> Result<Option<(DateTime<Utc>, i64)>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT created_at, amount_minor
        FROM portfolio_audit_log
        WHERE owner_id = $1 AND currency = $2
        ORDER BY created_at ASC
        LIMIT 1
        "#,
        owner_id.as_uuid(),
        currency
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.and_then(|r| r.created_at.map(|c| (c, r.amount_minor))))
}
