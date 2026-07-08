use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};

use crate::shared::{not_found, unauthorized, HttpError};
use crate::{app_state::AppState, transaction::types::Transaction};

/// API endpoint to retrieve a transaction by its external ID.
pub async fn get_transaction_by_external_id_api(
    State(state): State<Arc<AppState>>,
    Path(external_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Transaction>, HttpError> {
    state.validate_allowed_passed_basic_auth(&headers)?;
    if !state.is_api_key_valid_across_any_chain(&headers) {
        state.validate_basic_auth_valid(&headers).map_err(|_| unauthorized(None))?;
    }

    let transaction = state
        .db
        .get_transaction_by_external_id(&external_id)
        .await?
        .ok_or(not_found("Transaction external id not found".to_string()))?;

    state
        .validate_auth_basic_or_api_key(&headers, &transaction.from, &transaction.chain_id)
        .map_err(|_| not_found("Transaction external id not found".to_string()))?;

    Ok(Json(transaction))
}
