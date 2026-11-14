use std::sync::Arc;

use axum::{Json, extract::State};
use shared::{
    BroadcastTransactionResponse, ChainTip, FeeEstimate, SignedTransactionRequest,
    TransactionStatusRequest, TransactionStatusResponse, TransactionValidation,
};
use truenorth_rpc::{decode_hash, validate_transaction_hex};

use crate::{AppState, api::query_index, error::ApiError};

pub(crate) async fn chain_tip(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ChainTip>, ApiError> {
    let tip = state.node.fetch_tip().await?;
    Ok(Json(ChainTip {
        height: tip.blocks as u64,
    }))
}

pub(crate) async fn fee_estimate(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FeeEstimate>, ApiError> {
    let estimate = state.node.estimate_fee_rate().await?;
    Ok(Json(FeeEstimate {
        atomic_units_per_vbyte: estimate.atomic_units_per_vbyte,
        target_blocks: estimate.target_blocks,
    }))
}

pub(crate) async fn validate_transaction(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SignedTransactionRequest>,
) -> Result<Json<TransactionValidation>, ApiError> {
    validate_transaction_hex(&request.transaction_hex)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let validation = state
        .node
        .test_transaction_acceptance(&request.transaction_hex)
        .await?;
    Ok(Json(TransactionValidation {
        allowed: validation.allowed,
        rejection_reason: validation.rejection_reason,
    }))
}

pub(crate) async fn broadcast_transaction(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SignedTransactionRequest>,
) -> Result<Json<BroadcastTransactionResponse>, ApiError> {
    validate_transaction_hex(&request.transaction_hex)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let transaction_id = state
        .node
        .broadcast_transaction(&request.transaction_hex)
        .await?;
    Ok(Json(BroadcastTransactionResponse { transaction_id }))
}

pub(crate) async fn transaction_status(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TransactionStatusRequest>,
) -> Result<Json<TransactionStatusResponse>, ApiError> {
    let id = decode_hash(&request.transaction_id)
        .map_err(|_| ApiError::bad_request("transaction ID must be 64 hexadecimal characters"))?;
    let known = query_index(state.clone(), move |reader| {
        reader.read_snapshot(&[])?.query_transaction_known(&id)
    })
    .await?;
    let known = if known {
        true
    } else {
        state
            .node
            .is_transaction_known(&request.transaction_id)
            .await?
    };
    Ok(Json(TransactionStatusResponse { known }))
}
