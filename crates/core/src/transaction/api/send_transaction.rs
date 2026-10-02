use super::types::TransactionSpeed;
use crate::middleware::policy::PolicyContext;
use crate::rate_limiting::RateLimiter;
use crate::relayer::{get_relayer, Relayer};
use crate::shared::utils::convert_blob_strings_to_blobs;
use crate::shared::{internal_server_error, not_found, unauthorized, HttpError};
use crate::{
    app_state::{AppState, NetworkValidateAction},
    rate_limiting::RateLimitOperation,
    relayer::RelayerId,
    shared::common_types::EvmAddress,
    transaction::{
        queue_system::TransactionToSend,
        types::{TransactionData, TransactionHash, TransactionId, TransactionValue},
    },
};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayTransactionRequest {
    pub to: EvmAddress,
    #[serde(default)]
    pub value: TransactionValue,
    #[serde(default)]
    pub data: TransactionData,
    pub speed: Option<TransactionSpeed>,
    /// Stable request identity within this relayer; identical retries return the stored transaction.
    #[serde(rename = "externalId", skip_serializing_if = "Option::is_none", default)]
    pub external_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub blobs: Option<Vec<String>>, // will overflow the stack if you use the Blob type directly
}

impl FromStr for RelayTransactionRequest {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendTransactionResult {
    pub id: TransactionId,
    pub hash: Option<TransactionHash>,
}

/// API endpoint to send a new transaction through a relayer.
pub async fn handle_send_transaction(
    State(state): State<Arc<AppState>>,
    Path(relayer_id): Path<RelayerId>,
    policy_context: PolicyContext,
    headers: HeaderMap,
    Json(transaction): Json<RelayTransactionRequest>,
) -> Result<Json<SendTransactionResult>, HttpError> {
    state.validate_allowed_passed_basic_auth(&headers)?;

    let relayer = get_relayer(&state.db, &state.cache, &relayer_id)
        .await?
        .ok_or(not_found("Relayer does not exist".to_string()))?;

    let result = send_transaction(relayer, transaction, &state, &headers, &policy_context).await?;

    Ok(Json(result))
}

pub async fn send_transaction(
    relayer: Relayer,
    transaction: RelayTransactionRequest,
    state: &Arc<AppState>,
    headers: &HeaderMap,
    policy_context: &PolicyContext,
) -> Result<SendTransactionResult, HttpError> {
    state.validate_auth_basic_or_api_key(headers, &relayer.address, &relayer.chain_id)?;
    state.validate_request_policy(policy_context, headers, &relayer.address, &relayer.chain_id)?;

    if state.relayer_internal_only.restricted(&relayer.address, &relayer.chain_id) {
        return Err(unauthorized(Some("Relayer can only be used internally".to_string())));
    }

    state.network_permission_validate(
        &relayer.address,
        &relayer.chain_id,
        &transaction.to,
        &transaction.value,
        NetworkValidateAction::Transaction,
    )?;

    // Check if blob transactions are enabled for this network
    if transaction.blobs.is_some() {
        let network_config =
            state.network_configs.iter().find(|n| n.chain_id == relayer.chain_id).ok_or_else(
                || internal_server_error(Some("Network configuration not found".to_string())),
            )?;

        if !network_config.enable_sending_blobs.unwrap_or(false) {
            return Err(internal_server_error(Some(
                "Blob transactions are not enabled for this network".to_string(),
            )));
        }
    }

    if let Some(stored) = state
        .db
        .find_admitted_transaction(
            &relayer.id,
            transaction.external_id.as_deref(),
            &crate::transaction::db::admission::request_digest(
                transaction.to,
                transaction.value,
                &transaction.data,
                transaction.speed.as_ref().unwrap_or(&TransactionSpeed::FAST),
            ),
        )
        .await?
    {
        return Ok(SendTransactionResult { id: stored.id, hash: stored.known_transaction_hash });
    }

    let rate_limit_reservation = RateLimiter::check_and_reserve_rate_limit(
        state,
        headers,
        &relayer.id,
        RateLimitOperation::Transaction,
    )
    .await?;

    let transaction_to_send = TransactionToSend::new(
        transaction.to,
        transaction.value,
        transaction.data.clone(),
        transaction.speed.clone(),
        convert_blob_strings_to_blobs(transaction.blobs)?,
        transaction.external_id,
    );

    let transaction = state
        .transactions_queues
        .lock()
        .await
        .add_transaction(&relayer.id, &transaction_to_send)
        .await?;

    let result =
        SendTransactionResult { id: transaction.id, hash: transaction.known_transaction_hash };

    if transaction.id == transaction_to_send.id {
        if let Some(reservation) = rate_limit_reservation {
            reservation.commit();
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};
    use serde_json::json;

    fn transaction_id() -> TransactionId {
        TransactionId::from_str("11111111-1111-4111-8111-111111111111").unwrap()
    }

    #[tokio::test]
    async fn direct_submission_returns_http_200_with_explicit_null_hash_when_pending() {
        let response =
            Json(SendTransactionResult { id: transaction_id(), hash: None }).into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            json!({
                "id": "11111111-1111-4111-8111-111111111111",
                "hash": null
            })
        );
    }

    #[tokio::test]
    async fn direct_submission_returns_http_200_with_known_hash() {
        let hash = TransactionHash::from_str(
            "0x2222222222222222222222222222222222222222222222222222222222222222",
        )
        .unwrap();
        let response =
            Json(SendTransactionResult { id: transaction_id(), hash: Some(hash) }).into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            json!({
                "id": "11111111-1111-4111-8111-111111111111",
                "hash": "0x2222222222222222222222222222222222222222222222222222222222222222"
            })
        );
    }

    struct AdmissionGasEstimator;

    #[async_trait::async_trait]
    impl crate::gas::BaseGasFeeEstimator for AdmissionGasEstimator {
        async fn get_gas_prices(
            &self,
            _: &crate::network::ChainId,
        ) -> Result<crate::gas::GasEstimatorResult, crate::gas::GasEstimatorError> {
            let price = crate::gas::GasPriceResult {
                max_fee: crate::gas::MaxFee::new(1),
                max_priority_fee: crate::gas::MaxPriorityFee::new(1),
                min_wait_time_estimate: None,
                max_wait_time_estimate: None,
            };
            Ok(crate::gas::GasEstimatorResult {
                slow: price.clone(),
                medium: price.clone(),
                fast: price.clone(),
                super_fast: price,
            })
        }
        fn is_chain_supported(&self, _: &crate::network::ChainId) -> bool {
            true
        }
    }

    fn admission_block() -> serde_json::Value {
        json!({
            "hash": format!("0x{}", "00".repeat(32)),
            "parentHash": format!("0x{}", "00".repeat(32)),
            "sha3Uncles": format!("0x{}", "00".repeat(32)),
            "miner": format!("0x{}", "00".repeat(20)),
            "stateRoot": format!("0x{}", "00".repeat(32)),
            "transactionsRoot": format!("0x{}", "00".repeat(32)),
            "receiptsRoot": format!("0x{}", "00".repeat(32)),
            "logsBloom": format!("0x{}", "00".repeat(256)),
            "difficulty": "0x0", "number": "0x0", "gasLimit": "0x1c9c380",
            "gasUsed": "0x0", "timestamp": "0x0", "extraData": "0x",
            "mixHash": format!("0x{}", "00".repeat(32)),
            "nonce": "0x0000000000000000", "transactions": [], "uncles": []
        })
    }

    fn admission_request(
        relayer_id: &RelayerId,
        body: &serde_json::Value,
    ) -> axum::http::Request<axum::body::Body> {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri(format!("/relayers/{relayer_id}/send"))
            .header("content-type", "application/json")
            .header("x-rrelayer-basic-auth-valid", "true")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        request.extensions_mut().insert(PolicyContext::empty());
        request
    }

    #[tokio::test]
    #[ignore = "requires a dedicated PostgreSQL database in DATABASE_URL"]
    async fn send_endpoint_external_id_retries_conflicts_and_concurrent_sends_create_once() {
        use crate::{
            app_state::{RelayersAllowedForRandom, RelayersInternalOnly},
            gas::{BlobGasOracleCache, GasOracleCache},
            network::ChainId,
            postgres::PostgresClient,
            provider::EvmProvider,
            shared::cache::Cache,
            transaction::queue_system::{types::TransactionRelayerSetup, TransactionsQueues},
            SafeProxyManager,
        };
        use std::collections::HashMap;
        use tokio::sync::Mutex;
        use tower::ServiceExt;

        let asserter = alloy::transports::mock::Asserter::new();
        for response in [
            json!("0x0"), // Queue startup nonce.
            json!("0x0"),
            json!("0x0"),
            json!("0x5208"),
            admission_block(),
            json!("0xffffffffffffffff"),
            json!("0x0"),
            json!("0x0"),
            json!("0x5208"),
            json!("0xffffffffffffffff"),
        ] {
            asserter.push_success(&response);
        }
        let provider = EvmProvider::mocked(
            asserter,
            Arc::new(crate::wallet::MnemonicWalletManager::new(
                "test test test test test test test test test test test junk",
            )),
            Arc::new(AdmissionGasEstimator),
            ChainId::new(1),
        );
        let db = PostgresClient::new().await.unwrap();
        crate::schema::apply_schema(&db).await.unwrap();
        let relayer_id = RelayerId::new();
        let address = EvmAddress::zero();
        let wallet_index: i32 = db.query_one("SELECT COALESCE(max(wallet_index), -1) + 1 FROM relayer.record WHERE wallet_index >= 0", &[]).await.unwrap().get(0);
        db.execute("INSERT INTO relayer.record (id, name, chain_id, address, wallet_index) VALUES ($1, 'send-test', 1, $2, $3)", &[&relayer_id, &address, &wallet_index]).await.unwrap();
        let relayer = Relayer {
            id: relayer_id,
            name: "send-test".to_string(),
            chain_id: ChainId::new(1),
            cloned_from_chain_id: None,
            address,
            wallet_index,
            max_gas_price: None,
            paused: false,
            eip_1559_enabled: true,
            created_at: chrono::Utc::now(),
            is_private_key: false,
        };
        let gas = Arc::new(Mutex::new(GasOracleCache::new()));
        let blobs = Arc::new(Mutex::new(BlobGasOracleCache::new()));
        let cache = Arc::new(Cache::new().await);
        let safe = Arc::new(SafeProxyManager::new(Vec::new()));
        let providers = Arc::new(vec![provider.clone()]);
        crate::gas::gas_oracle(providers.clone(), gas.clone()).await;
        // Construct queues without starting broadcast workers: only admission is under test.
        let queues = TransactionsQueues::new(
            vec![TransactionRelayerSetup::new(
                relayer,
                provider,
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                2,
            )],
            gas.clone(),
            blobs.clone(),
            cache.clone(),
            None,
            safe.clone(),
        )
        .await
        .unwrap();
        let state = Arc::new(AppState {
            started_at: chrono::Utc::now(),
            db: Arc::new(db),
            evm_providers: providers,
            gas_oracle_cache: gas,
            blob_gas_oracle_cache: blobs,
            transactions_queues: Arc::new(Mutex::new(queues)),
            cache,
            webhook_manager: None,
            user_rate_limiter: None,
            rate_limit_config: None,
            relayer_creation_mutex: Arc::new(Mutex::new(())),
            safe_proxy_manager: safe,
            relayer_internal_only: Arc::new(RelayersInternalOnly::new(Vec::new())),
            relayers_allowed_for_random: Arc::new(RelayersAllowedForRandom::new(HashMap::new())),
            network_permissions: Arc::new(Vec::new()),
            api_keys: Arc::new(Vec::new()),
            network_configs: Arc::new(Vec::new()),
            private_key_only_networks: Arc::new(Vec::new()),
        });
        let app = super::super::create_transactions_routes().with_state(state.clone());
        let original =
            json!({"to":"0x1111111111111111111111111111111111111111", "externalId":"endpoint-key"});
        let response =
            app.clone().oneshot(admission_request(&relayer_id, &original)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: SendTransactionResult =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(accepted.hash.is_none());
        let mut retry = original.clone();
        retry["speed"] = json!("FAST");
        let response = app.clone().oneshot(admission_request(&relayer_id, &retry)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let stored: SendTransactionResult =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(stored.id, accepted.id);
        for (field, value) in [
            ("to", json!("0x2222222222222222222222222222222222222222")),
            ("value", json!("0x1")),
            ("data", json!("0x0102")),
            ("speed", json!("SUPER")),
        ] {
            let mut changed = original.clone();
            changed[field] = value;
            let response =
                app.clone().oneshot(admission_request(&relayer_id, &changed)).await.unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT, "{field}");
        }
        let count: i64 = state
            .db
            .query_one(
                "SELECT count(*) FROM relayer.transaction WHERE relayer_id = $1",
                &[&relayer_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 1);
        let stored = state.db.get_transaction(&accepted.id).await.unwrap().unwrap();
        assert_eq!(stored.value, TransactionValue::zero());
        assert_eq!(stored.data, TransactionData::empty());
        assert_eq!(stored.speed, TransactionSpeed::FAST);
        assert_eq!(
            state.transactions_queues.lock().await.pending_transactions_count(&relayer_id).await,
            1
        );

        let mut concurrent = original.clone();
        concurrent["externalId"] = json!("endpoint-race");
        let (left, right) = tokio::join!(
            app.clone().oneshot(admission_request(&relayer_id, &concurrent)),
            app.clone().oneshot(admission_request(&relayer_id, &concurrent)),
        );
        let left = left.unwrap();
        let right = right.unwrap();
        assert_eq!(left.status(), StatusCode::OK);
        assert_eq!(right.status(), StatusCode::OK);
        let left: SendTransactionResult =
            serde_json::from_slice(&to_bytes(left.into_body(), usize::MAX).await.unwrap()).unwrap();
        let right: SendTransactionResult =
            serde_json::from_slice(&to_bytes(right.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(left.id, right.id);
        assert_eq!(
            state.transactions_queues.lock().await.pending_transactions_count(&relayer_id).await,
            2
        );
        let count: i64 = state
            .db
            .query_one(
                "SELECT count(*) FROM relayer.transaction WHERE relayer_id = $1",
                &[&relayer_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 2);
        let lookup = axum::http::Request::builder()
            .uri(format!("/relayers/{relayer_id}/external/endpoint-key"))
            .header("x-rrelayer-basic-auth-valid", "true")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.clone().oneshot(lookup).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let found: crate::transaction::types::Transaction =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(found.id, accepted.id);
        let missing = axum::http::Request::builder()
            .uri(format!("/relayers/{relayer_id}/external/missing"))
            .header("x-rrelayer-basic-auth-valid", "true")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(app.clone().oneshot(missing).await.unwrap().status(), StatusCode::NOT_FOUND);
        let unauthenticated = axum::http::Request::builder()
            .uri(format!("/relayers/{relayer_id}/external/endpoint-key"))
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(app.oneshot(unauthenticated).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    }
}
