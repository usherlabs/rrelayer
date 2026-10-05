use super::builders::build_transaction_from_transaction_view;
use crate::{
    postgres::{PostgresClient, PostgresError},
    relayer::RelayerId,
    shared::common_types::EvmAddress,
    transaction::{
        queue_system::types::AddTransactionError,
        types::{Transaction, TransactionData, TransactionSpeed, TransactionValue},
    },
};
use sha2::{Digest, Sha256};

/// Version 1 encoding: ASCII domain, 20 address bytes, 32 big-endian value bytes,
/// 8 big-endian data-length bytes, data bytes, then one speed byte
/// (SLOW=0, MEDIUM=1, FAST=2, SUPER=3). Omitted speed is normalized to FAST
/// before hashing. Live fee bumps and replacement payloads never change this digest.
pub(crate) fn request_digest(
    to: EvmAddress,
    value: TransactionValue,
    data: &TransactionData,
    speed: &TransactionSpeed,
) -> Vec<u8> {
    let data = data.clone().into_inner();
    let mut digest = Sha256::new();
    digest.update(b"rrelayer-admission-v1");
    digest.update(to.into_address().as_slice());
    digest.update(value.into_inner().to_be_bytes::<32>());
    digest.update((data.len() as u64).to_be_bytes());
    digest.update(data);
    digest.update([match speed {
        TransactionSpeed::SLOW => 0,
        TransactionSpeed::MEDIUM => 1,
        TransactionSpeed::FAST => 2,
        TransactionSpeed::SUPER => 3,
    }]);
    digest.finalize().to_vec()
}

impl PostgresClient {
    pub(crate) async fn find_admitted_transaction(
        &self,
        relayer_id: &RelayerId,
        external_id: Option<&str>,
        digest: &[u8],
    ) -> Result<Option<Transaction>, AddTransactionError> {
        let Some(external_id) = external_id else { return Ok(None) };
        let row = self
            .query_one_or_none(
                "SELECT * FROM relayer.transaction WHERE relayer_id = $1 AND external_id = $2",
                &[relayer_id, &external_id],
            )
            .await
            .map_err(AddTransactionError::CouldNotSaveTransactionDb)?;
        let Some(row) = row else { return Ok(None) };
        if row.get::<_, Option<Vec<u8>>>("request_digest").as_deref() != Some(digest) {
            return Err(AddTransactionError::ExternalIdConflict(external_id.to_string()));
        }
        Ok(Some(build_transaction_from_transaction_view(&row)))
    }

    pub(crate) async fn save_admitted_transaction(
        &mut self,
        relayer_id: &RelayerId,
        transaction: &Transaction,
    ) -> Result<Transaction, AddTransactionError> {
        match self.save_transaction(relayer_id, transaction).await {
            Ok(()) => Ok(transaction.clone()),
            Err(error) => self.resolve_admission_insert_error(relayer_id, transaction, error).await,
        }
    }

    pub(crate) async fn resolve_admission_insert_error(
        &self,
        relayer_id: &RelayerId,
        transaction: &Transaction,
        error: PostgresError,
    ) -> Result<Transaction, AddTransactionError> {
        let external_id_race = matches!(&error, PostgresError::PgError(error)
                    if error.as_db_error().is_some_and(|error|
                        error.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
                        && error.constraint() == Some("idx_transaction_relayer_external_id")));
        if external_id_race {
            if let Some(stored) = self
                .find_admitted_transaction(
                    relayer_id,
                    transaction.external_id.as_deref(),
                    &request_digest(
                        transaction.to,
                        transaction.value,
                        &transaction.data,
                        &transaction.speed,
                    ),
                )
                .await?
            {
                return Ok(stored);
            }
        }
        Err(AddTransactionError::CouldNotSaveTransactionDb(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        gas::GasLimit,
        network::ChainId,
        schema::apply_schema,
        shared::{common_types::PagingContext, HttpError},
        transaction::{
            api::SendTransactionResult,
            types::{TransactionId, TransactionNonce, TransactionStatus},
        },
    };
    use axum::{http::StatusCode, response::IntoResponse, Json};
    use chrono::Utc;

    fn transaction_for(relayer_id: RelayerId, external_id: &str) -> Transaction {
        let now = Utc::now();
        Transaction {
            id: TransactionId::new(),
            relayer_id,
            to: EvmAddress::zero(),
            from: EvmAddress::zero(),
            value: TransactionValue::zero(),
            data: TransactionData::empty(),
            nonce: TransactionNonce::new(0),
            chain_id: ChainId::new(1),
            gas_limit: Some(GasLimit::new(21_000)),
            status: TransactionStatus::PENDING,
            blobs: None,
            known_transaction_hash: None,
            queued_at: now,
            expires_at: now,
            sent_at: None,
            confirmed_at: None,
            sent_with_gas: None,
            sent_with_blob_gas: None,
            mined_at: None,
            mined_at_block_number: None,
            speed: TransactionSpeed::FAST,
            sent_with_max_priority_fee_per_gas: None,
            sent_with_max_fee_per_gas: None,
            is_noop: false,
            external_id: Some(external_id.to_string()),
            cancelled_by_transaction_id: None,
            failed_reason: None,
        }
    }

    #[test]
    fn admission_digest_matches_version_one_canonical_vector() {
        assert_eq!(
            hex::encode(request_digest(
                EvmAddress::zero(),
                TransactionValue::new(alloy::primitives::U256::from(7)),
                &TransactionData::from(vec![1, 2]),
                &TransactionSpeed::FAST,
            )),
            "e6c12dbb17adf8d86ad61391f105206db91498dc677b633d58a186d5c1f8a6c8"
        );
    }

    #[tokio::test]
    #[ignore = "requires a dedicated PostgreSQL database in DATABASE_URL"]
    async fn external_id_admission_retries_conflicts_and_concurrency_preserve_one_row() {
        let mut db = PostgresClient::new().await.unwrap();
        apply_schema(&db).await.unwrap();
        apply_schema(&db).await.unwrap();
        let relayer_id = RelayerId::new();
        db.execute("INSERT INTO relayer.record (id, name, chain_id, wallet_index, is_private_key) VALUES ($1, 'admission-test', 1, -1, TRUE)", &[&relayer_id]).await.unwrap();
        let original = transaction_for(relayer_id, "retry-key");
        let accepted = db.save_admitted_transaction(&relayer_id, &original).await.unwrap();
        assert_eq!(accepted.id, original.id);
        let response =
            Json(SendTransactionResult { id: accepted.id, hash: accepted.known_transaction_hash })
                .into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let mut retry = original.clone();
        retry.id = TransactionId::new();
        let stored = db.save_admitted_transaction(&relayer_id, &retry).await.unwrap();
        assert_eq!(stored.id, original.id);
        let mut changed = retry.clone();
        changed.value = TransactionValue::new(alloy::primitives::U256::from(1));
        let error: HttpError =
            db.save_admitted_transaction(&relayer_id, &changed).await.unwrap_err().into();
        assert_eq!(error.0, StatusCode::CONFLICT);
        assert!(error.1.contains("External ID"));
        let rows =
            db.get_transactions_for_relayer(&relayer_id, &PagingContext::new(10, 0)).await.unwrap();
        assert_eq!(rows.items.len(), 1);
        assert_eq!(rows.items[0].id, original.id);
        assert_eq!(rows.items[0].value, TransactionValue::zero());
        let audit_count: i64 = db
            .query_one(
                "SELECT count(*) FROM relayer.transaction_audit_log WHERE relayer_id = $1",
                &[&relayer_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(audit_count, 1);

        // Live replacement/fee changes must not redefine the original request.
        let mut replaced = original.clone();
        replaced.value = TransactionValue::new(alloy::primitives::U256::from(2));
        replaced.speed = TransactionSpeed::SUPER;
        replaced.status = TransactionStatus::REPLACED;
        db.transaction_update(&replaced).await.unwrap();
        let stored = db
            .find_admitted_transaction(
                &relayer_id,
                Some("retry-key"),
                &request_digest(original.to, original.value, &original.data, &original.speed),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.id, original.id);
        assert_eq!(stored.value, replaced.value);

        let first = transaction_for(relayer_id, "race-key");
        let mut second = first.clone();
        second.id = TransactionId::new();
        let mut other_db = PostgresClient::new().await.unwrap();
        let (left, right) = tokio::join!(
            db.save_admitted_transaction(&relayer_id, &first),
            other_db.save_admitted_transaction(&relayer_id, &second),
        );
        assert_eq!(left.unwrap().id, right.unwrap().id);
        let count: i64 = db.query_one("SELECT count(*) FROM relayer.transaction WHERE relayer_id = $1 AND external_id = 'race-key'", &[&relayer_id]).await.unwrap().get(0);
        assert_eq!(count, 1);
        let audit_count: i64 = db.query_one("SELECT count(*) FROM relayer.transaction_audit_log WHERE relayer_id = $1 AND external_id = 'race-key'", &[&relayer_id]).await.unwrap().get(0);
        assert_eq!(audit_count, 1);

        let other_relayer = RelayerId::new();
        db.execute("INSERT INTO relayer.record (id, name, chain_id, wallet_index, is_private_key) VALUES ($1, 'admission-test', 1, -2, TRUE)", &[&other_relayer]).await.unwrap();
        let other = transaction_for(other_relayer, "retry-key");
        db.save_admitted_transaction(&other_relayer, &other).await.unwrap();
        assert_eq!(
            db.get_transaction_by_external_id_for_relayer(&relayer_id, "retry-key")
                .await
                .unwrap()
                .unwrap()
                .id,
            original.id
        );
        assert!(db
            .get_transaction_by_external_id_for_relayer(&relayer_id, "missing")
            .await
            .unwrap()
            .is_none());
        let error: HttpError =
            db.get_transaction_by_external_id("retry-key").await.unwrap_err().into();
        assert_eq!(error.0, StatusCode::CONFLICT);

        db.execute(
            "UPDATE relayer.transaction SET request_digest = NULL WHERE id = $1",
            &[&original.id],
        )
        .await
        .unwrap();
        let error: HttpError = db
            .find_admitted_transaction(
                &relayer_id,
                Some("retry-key"),
                &request_digest(original.to, original.value, &original.data, &original.speed),
            )
            .await
            .unwrap_err()
            .into();
        assert_eq!(error.0, StatusCode::CONFLICT);
    }
}
