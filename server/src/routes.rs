use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use indexer::decode_addresses;
use shared::{
    AddressActivityRequest, AddressActivityResponse, BroadcastTransactionResponse, ChainTip,
    DEFAULT_UTXO_PAGE_SIZE, FeeEstimate, FeeEstimateV2, IndexStatus, ServerInfo,
    SignedTransactionRequest, TransactionHistory, TransactionHistoryRequest,
    TransactionStatusRequest, TransactionStatusResponse, TransactionValidation, UtxoRequest,
    UtxoResponse,
};
use truenorth_rpc::{decode_hash, validate_transaction_hex};

use crate::{AppState, error::ApiError};

const DEFAULT_HISTORY_PAGE_SIZE: u16 = 100;

// --- Server metadata & readiness ---

pub(crate) async fn server_info(State(state): State<Arc<AppState>>) -> Json<ServerInfo> {
    Json(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        network: state.network.clone(),
    })
}

pub(crate) async fn readiness(
    State(state): State<Arc<AppState>>,
) -> Result<(StatusCode, Json<IndexStatus>), ApiError> {
    let status = state.read_index(|reader| reader.query_status()).await?;
    let http_status = if status.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    Ok((http_status, Json(status)))
}

// --- Chain & fee endpoints (served from truenorthd) ---

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
        atomic_units_per_vbyte: estimate.atomic_units_per_kvbyte.div_ceil(1000),
        target_blocks: estimate.target_blocks,
    }))
}

pub(crate) async fn fee_estimate_v2(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FeeEstimateV2>, ApiError> {
    let estimate = state.node.estimate_fee_rate().await?;
    Ok(Json(FeeEstimateV2 {
        atomic_units_per_kvbyte: estimate.atomic_units_per_kvbyte,
        target_blocks: estimate.target_blocks,
    }))
}

// --- Indexed wallet queries (served from local SQLite index) ---

pub(crate) async fn address_activity(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AddressActivityRequest>,
) -> Result<Json<AddressActivityResponse>, ApiError> {
    let scripts = decode_addresses(&request.addresses, &state.network)?;

    let response = state
        .read_index(move |reader| {
            let snapshot = reader.read_snapshot(&scripts)?;
            Ok(AddressActivityResponse {
                used_addresses: snapshot.query_script_activity()?,
                index: snapshot.status.clone(),
            })
        })
        .await?;

    Ok(Json(response))
}

pub(crate) async fn utxos(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UtxoRequest>,
) -> Result<Json<UtxoResponse>, ApiError> {
    let scripts = decode_addresses(&request.addresses, &state.network)?;
    let limit = request.limit.unwrap_or(DEFAULT_UTXO_PAGE_SIZE);

    let response = state
        .read_index(move |reader| {
            let snapshot = reader.read_snapshot(&scripts)?;
            snapshot.query_utxos(request.cursor.as_deref(), limit)
        })
        .await?;

    Ok(Json(response))
}

pub(crate) async fn transactions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TransactionHistoryRequest>,
) -> Result<Json<TransactionHistory>, ApiError> {
    let scripts = decode_addresses(&request.addresses, &state.network)?;
    let limit = request.limit.unwrap_or(DEFAULT_HISTORY_PAGE_SIZE);

    let response = state
        .read_index(move |reader| {
            let snapshot = reader.read_snapshot(&scripts)?;
            snapshot.query_transaction_history(request.cursor.as_deref(), limit)
        })
        .await?;

    Ok(Json(response))
}

// --- Transaction validation, broadcast, and status ---

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

    match state
        .node
        .broadcast_transaction(&request.transaction_hex)
        .await
    {
        Ok(transaction_id) => Ok(Json(BroadcastTransactionResponse { transaction_id })),
        Err(error) if is_missing_inputs_rejection(&error) => {
            // If all outputs of an already-confirmed transaction were later spent,
            // truenorthd reports missing inputs instead of "already in chain".
            // Treat a re-broadcast as success only if this exact transaction ID
            // is already confirmed in our fresh index snapshot.
            if let Some(transaction_id) =
                find_confirmed_transaction_id(&state, &request.transaction_hex).await?
            {
                return Ok(Json(BroadcastTransactionResponse { transaction_id }));
            }
            Err(ApiError::from_broadcast_error(error))
        }
        Err(error) => Err(ApiError::from_broadcast_error(error)),
    }
}

pub(crate) async fn transaction_status(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TransactionStatusRequest>,
) -> Result<Json<TransactionStatusResponse>, ApiError> {
    let id = decode_hash(&request.transaction_id)
        .map_err(|_| ApiError::bad_request("transaction ID must be 64 hexadecimal characters"))?;

    let indexed_status = state
        .read_index(move |reader| reader.read_snapshot(&[])?.query_transaction_status(&id))
        .await?;
    if indexed_status.known {
        return Ok(Json(indexed_status));
    }

    // Fall back to the node for transactions not yet observed by the indexer.
    let node_status = state
        .node
        .fetch_transaction_status(&request.transaction_id)
        .await?;

    let (known, block_height) = match node_status {
        truenorth_rpc::TransactionStatus::Unknown => (false, None),
        truenorth_rpc::TransactionStatus::Pending => (true, None),
        truenorth_rpc::TransactionStatus::Confirmed { block_height } => (true, Some(block_height)),
    };

    Ok(Json(TransactionStatusResponse {
        known,
        block_height,
    }))
}

fn is_missing_inputs_rejection(error: &truenorth_rpc::Error) -> bool {
    matches!(
        error,
        truenorth_rpc::Error::Remote {
            method,
            code: -25,
            message,
        } if method == "sendrawtransaction" && message == "bad-txns-inputs-missingorspent"
    )
}

async fn find_confirmed_transaction_id(
    state: &AppState,
    transaction_hex: &str,
) -> Result<Option<String>, ApiError> {
    let transaction_id = state.node.decode_transaction_id(transaction_hex).await?;
    let id = decode_hash(&transaction_id)?;
    let status = state
        .read_index(move |reader| reader.read_snapshot(&[])?.query_transaction_status(&id))
        .await?;

    if status.block_height.is_some() {
        Ok(Some(transaction_id))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "routes_tests.rs"]
mod tests;
