use crate::postgres::{PostgresClient, PostgresError};

/// Existing duplicates must be resolved by the operator: choosing a winner could
/// hide an already broadcast transaction. Legacy rows have no trustworthy
/// original request digest and cannot accept idempotent retries.
pub async fn apply_v1_0_5_schema(client: &PostgresClient) -> Result<(), PostgresError> {
    client
        .batch_execute(
            r#"
        ALTER TABLE relayer.transaction ADD COLUMN IF NOT EXISTS request_digest BYTEA;
        ALTER TABLE relayer.transaction_audit_log ADD COLUMN IF NOT EXISTS request_digest BYTEA;
        CREATE UNIQUE INDEX IF NOT EXISTS idx_transaction_relayer_external_id
        ON relayer.transaction(relayer_id, external_id) WHERE external_id IS NOT NULL;
    "#,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a dedicated PostgreSQL database in DATABASE_URL"]
    async fn external_id_migration_refuses_duplicates_without_rewriting_transactions() {
        let admin = PostgresClient::new().await.unwrap();
        let name = format!("migration_{}", uuid::Uuid::new_v4().simple());
        admin.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();
        let original_url = std::env::var("DATABASE_URL").unwrap();
        let mut url = reqwest::Url::parse(&original_url).unwrap();
        url.set_path(&name);
        std::env::set_var("DATABASE_URL", url.as_str());
        let db = PostgresClient::new().await.unwrap();
        std::env::set_var("DATABASE_URL", original_url);
        crate::schema::v1_0_0::apply_v1_0_0_schema(&db).await.unwrap();
        let relayer = crate::relayer::RelayerId::new();
        db.execute("INSERT INTO relayer.record (id, name, chain_id, wallet_index) VALUES ($1, 'migration-test', 1, 0)", &[&relayer]).await.unwrap();
        let first = crate::transaction::types::TransactionId::new();
        let second = crate::transaction::types::TransactionId::new();
        let insert = r#"INSERT INTO relayer.transaction
            (id, relayer_id, "to", "from", nonce, value, data, chain_id, speed, status, expires_at, external_id)
            VALUES ($1, $2, decode(repeat('00',20),'hex'), decode(repeat('00',20),'hex'), 0, 0,
                    decode('','hex'), 1, 'FAST', 'PENDING', NOW(), 'legacy-duplicate')"#;
        db.execute(insert, &[&first, &relayer]).await.unwrap();
        db.execute(insert, &[&second, &relayer]).await.unwrap();
        let error = apply_v1_0_5_schema(&db).await.unwrap_err();
        assert!(matches!(error, PostgresError::PgError(error)
            if error.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION)));
        let rows = db.query("SELECT id FROM relayer.transaction ORDER BY id", &[]).await.unwrap();
        let mut ids: Vec<crate::transaction::types::TransactionId> =
            rows.iter().map(|row| row.get(0)).collect();
        ids.sort_by_key(|id| id.to_string());
        let mut expected = vec![first, second];
        expected.sort_by_key(|id| id.to_string());
        assert_eq!(ids, expected);
        // An explicit correlation-ID repair lets the same forward migration finish.
        db.execute(
            "UPDATE relayer.transaction SET external_id = 'legacy-distinct' WHERE id = $1",
            &[&second],
        )
        .await
        .unwrap();
        apply_v1_0_5_schema(&db).await.unwrap();
        apply_v1_0_5_schema(&db).await.unwrap();
        let digest: Option<Vec<u8>> = db
            .query_one("SELECT request_digest FROM relayer.transaction WHERE id = $1", &[&first])
            .await
            .unwrap()
            .get(0);
        assert!(digest.is_none());
        drop(db);
        admin.batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)")).await.unwrap();
    }
}
