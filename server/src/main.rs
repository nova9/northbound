mod config;
mod rpc;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use config::ServerConfig;
use reqwest::StatusCode;
use rpc::NodeRpcClient;
use shared::{
    AddressActivityRequest, AddressActivityResponse, BroadcastTransactionResponse, ChainTip,
    FeeEstimate, SignedTransactionRequest, TransactionHistory, TransactionHistoryRequest,
    TransactionStatusRequest, TransactionStatusResponse, TransactionValidation, UtxoRequest,
    UtxoResponse,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig::from_environment()?;
    let rpc_client = NodeRpcClient::new(config.rpc_url, config.rpc_cookie_path, config.network);

    let app = Router::new()
        .route("/v1/chain/tip", get(chain_tip))
        .route("/v1/fees/estimate", get(fee_estimate))
        .route("/v1/addresses/activity", post(address_activity))
        .route("/v1/transactions", post(transactions))
        .route("/v1/transactions/validate", post(validate_transaction))
        .route("/v1/transactions/broadcast", post(broadcast_transaction))
        .route("/v1/transactions/status", post(transaction_status))
        .route("/v1/utxos", post(utxos))
        .layer(DefaultBodyLimit::max(1_000_000))
        .with_state(Arc::new(rpc_client));

    let listener = tokio::net::TcpListener::bind(config.bind_address).await?;

    println!("Server listening on http://{}", config.bind_address);

    axum::serve(listener, app).await?;

    Ok(())
}

async fn transaction_status(
    State(rpc_client): State<Arc<NodeRpcClient>>,
    Json(request): Json<TransactionStatusRequest>,
) -> Result<Json<TransactionStatusResponse>, (StatusCode, String)> {
    rpc::validate_transaction_id(&request.transaction_id)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let known = rpc_client
        .is_transaction_known(&request.transaction_id)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(Json(TransactionStatusResponse { known }))
}

async fn fee_estimate(
    State(rpc_client): State<Arc<NodeRpcClient>>,
) -> Result<Json<FeeEstimate>, (StatusCode, String)> {
    let estimate = rpc_client
        .estimate_fee_rate()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(Json(estimate))
}

async fn address_activity(
    State(rpc_client): State<Arc<NodeRpcClient>>,
    Json(request): Json<AddressActivityRequest>,
) -> Result<Json<AddressActivityResponse>, (StatusCode, String)> {
    rpc::validate_addresses(&request.addresses)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let used_addresses = rpc_client
        .get_used_addresses(&request.addresses)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(Json(AddressActivityResponse { used_addresses }))
}

async fn validate_transaction(
    State(rpc_client): State<Arc<NodeRpcClient>>,
    Json(request): Json<SignedTransactionRequest>,
) -> Result<Json<TransactionValidation>, (StatusCode, String)> {
    rpc::validate_transaction_hex(&request.transaction_hex)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let validation = rpc_client
        .test_transaction_acceptance(&request.transaction_hex)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(Json(validation))
}

async fn broadcast_transaction(
    State(rpc_client): State<Arc<NodeRpcClient>>,
    Json(request): Json<SignedTransactionRequest>,
) -> Result<Json<BroadcastTransactionResponse>, (StatusCode, String)> {
    rpc::validate_transaction_hex(&request.transaction_hex)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let transaction_id = rpc_client
        .broadcast_transaction(&request.transaction_hex)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(Json(BroadcastTransactionResponse { transaction_id }))
}

async fn chain_tip(
    State(rpc_client): State<Arc<NodeRpcClient>>,
) -> Result<Json<ChainTip>, (StatusCode, String)> {
    let height = rpc_client
        .get_block_height()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(Json(ChainTip { height }))
}

async fn transactions(
    State(rpc_client): State<Arc<NodeRpcClient>>,
    Json(request): Json<TransactionHistoryRequest>,
) -> Result<Json<TransactionHistory>, (StatusCode, String)> {
    rpc::validate_addresses(&request.addresses)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let transactions = rpc_client
        .get_transactions_for_addresses(&request.addresses)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(Json(TransactionHistory { transactions }))
}

async fn utxos(
    State(rpc_client): State<Arc<NodeRpcClient>>,
    Json(request): Json<UtxoRequest>,
) -> Result<Json<UtxoResponse>, (StatusCode, String)> {
    rpc::validate_addresses(&request.addresses)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let utxos = rpc_client
        .get_utxos_for_addresses(&request.addresses)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(Json(UtxoResponse { utxos }))
}
