use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};

use crate::relayer::{get_relayer, RelayerId};
use crate::shared::{not_found, HttpError};
use crate::{app_state::AppState, transaction::types::Transaction};

/// Relayer scope makes the external identity unambiguous and authenticates
/// access even when the requested transaction does not exist.
pub async fn get_transaction_by_external_id_for_relayer_api(
    State(state): State<Arc<AppState>>,
    Path((relayer_id, external_id)): Path<(RelayerId, String)>,
    headers: HeaderMap,
) -> Result<Json<Transaction>, HttpError> {
    state.validate_allowed_passed_basic_auth(&headers)?;
    let relayer = get_relayer(&state.db, &state.cache, &relayer_id)
        .await?
        .ok_or_else(|| not_found("Relayer does not exist".to_string()))?;
    state.validate_auth_basic_or_api_key(&headers, &relayer.address, &relayer.chain_id)?;
    let transaction = state
        .db
        .get_transaction_by_external_id_for_relayer(&relayer_id, &external_id)
        .await?
        .ok_or_else(|| not_found("Transaction does not exist".to_string()))?;
    Ok(Json(transaction))
}

/// API endpoint to retrieve a transaction by its external ID.
pub async fn get_transaction_by_external_id_api(
    State(state): State<Arc<AppState>>,
    Path(external_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Option<Transaction>>, HttpError> {
    state.validate_allowed_passed_basic_auth(&headers)?;

    let result = state.db.get_transaction_by_external_id(&external_id).await?;
    if let Some(transaction) = &result {
        state.validate_auth_basic_or_api_key(&headers, &transaction.from, &transaction.chain_id)?;
    }

    Ok(Json(result))
}
