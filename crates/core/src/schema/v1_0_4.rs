use crate::postgres::{PostgresClient, PostgresError};

/// Adds downstream integrity indexes for ordered attempt recovery and stable
/// live normal-wallet namespaces. Both indexes are additive and idempotent.
pub async fn apply_v1_0_4_schema(client: &PostgresClient) -> Result<(), PostgresError> {
    let schema_sql = r#"
        CREATE INDEX IF NOT EXISTS idx_transaction_audit_attempt_lookup
        ON relayer.transaction_audit_log(id, history_id)
        WHERE hash IS NOT NULL AND sent_at IS NOT NULL;

        CREATE UNIQUE INDEX IF NOT EXISTS idx_relayer_live_normal_wallet_namespace
        ON relayer.record(chain_id, wallet_index)
        WHERE deleted = FALSE AND is_private_key = FALSE AND wallet_index >= 0;
    "#;

    client.batch_execute(schema_sql).await?;
    Ok(())
}
