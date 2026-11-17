use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use indexer::{IndexReader, decode_addresses};
use shared::{
    AddressActivityRequest, AddressActivityResponse, DEFAULT_UTXO_PAGE_SIZE, IndexStatus,
    ServerInfo, TransactionHistory, TransactionHistoryRequest, UtxoRequest, UtxoResponse,
};

use crate::{AppState, error::ApiError};

pub(crate) async fn server_info(State(state): State<Arc<AppState>>) -> Json<ServerInfo> {
    Json(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        network: state.network.clone(),
    })
}

pub(crate) async fn query_index<T, F>(state: Arc<AppState>, query: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&mut IndexReader) -> indexer::Result<T> + Send + 'static,
{
    let permit = state
        .query_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::unavailable("The index is busy; retry shortly."))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut reader = IndexReader::open(&state.database_path, &state.network)?;
        query(&mut reader)
    })
    .await
    .map_err(|error| {
        eprintln!("Index query task failed: {error}");
        ApiError::unavailable("Could not read the index; retry shortly.")
    })?
    .map_err(ApiError::from)
}

pub(crate) async fn readiness(
    State(state): State<Arc<AppState>>,
) -> Result<(StatusCode, Json<IndexStatus>), ApiError> {
    let status = query_index(state, |reader| reader.query_status()).await?;
    let code = if status.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    Ok((code, Json(status)))
}

pub(crate) async fn address_activity(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AddressActivityRequest>,
) -> Result<Json<AddressActivityResponse>, ApiError> {
    let scripts = decode_addresses(&request.addresses, &state.network)?;
    let response = query_index(state, move |reader| {
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
    let response = query_index(state, move |reader| {
        let snapshot = reader.read_snapshot(&scripts)?;
        snapshot.query_utxos(
            request.cursor.as_deref(),
            request.limit.unwrap_or(DEFAULT_UTXO_PAGE_SIZE),
        )
    })
    .await?;
    Ok(Json(response))
}

pub(crate) async fn transactions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TransactionHistoryRequest>,
) -> Result<Json<TransactionHistory>, ApiError> {
    let scripts = decode_addresses(&request.addresses, &state.network)?;
    let response = query_index(state, move |reader| {
        let snapshot = reader.read_snapshot(&scripts)?;
        snapshot.query_transaction_history(request.cursor.as_deref(), request.limit.unwrap_or(100))
    })
    .await?;
    Ok(Json(response))
}
