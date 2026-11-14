mod api;
pub mod config;
mod error;
mod node_routes;

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use tokio::sync::Semaphore;
use truenorth_rpc::NodeClient;

pub struct AppState {
    node: NodeClient,
    database_path: PathBuf,
    network: String,
    query_slots: Arc<Semaphore>,
}

impl AppState {
    pub fn new(node: NodeClient, database_path: PathBuf, network: String) -> Self {
        Self {
            node,
            database_path,
            network,
            query_slots: Arc::new(Semaphore::new(16)),
        }
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ready", get(api::readiness))
        .route("/v1/chain/tip", get(node_routes::chain_tip))
        .route("/v1/fees/estimate", get(node_routes::fee_estimate))
        .route("/v1/addresses/activity", post(api::address_activity))
        .route("/v1/transactions", post(api::transactions))
        .route(
            "/v1/transactions/validate",
            post(node_routes::validate_transaction),
        )
        .route(
            "/v1/transactions/broadcast",
            post(node_routes::broadcast_transaction),
        )
        .route(
            "/v1/transactions/status",
            post(node_routes::transaction_status),
        )
        .route("/v1/utxos", post(api::utxos))
        .layer(DefaultBodyLimit::max(1_000_000))
        .layer(axum::middleware::map_response(error::format_response))
        .with_state(state)
}
