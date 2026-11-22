pub mod config;
mod error;
pub mod mining;
mod routes;
#[cfg(test)]
mod test_support;

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use indexer::IndexReader;
use tokio::sync::Semaphore;
use truenorth_rpc::NodeClient;

use crate::error::ApiError;

const MAX_CONCURRENT_INDEX_QUERIES: usize = 16;
const MAX_REQUEST_BODY_BYTES: usize = 1_000_000;

pub struct AppState {
    pub(crate) node: NodeClient,
    pub(crate) network: String,
    database_path: PathBuf,
    query_slots: Arc<Semaphore>,
}

impl AppState {
    pub fn new(node: NodeClient, database_path: PathBuf, network: String) -> Self {
        Self {
            node,
            network,
            database_path,
            query_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_INDEX_QUERIES)),
        }
    }

    /// Runs a read-only SQLite query on a blocking thread, bounded to at most
    /// `MAX_CONCURRENT_INDEX_QUERIES` concurrent readers.
    pub(crate) async fn read_index<T, F>(&self, query: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&mut IndexReader) -> indexer::Result<T> + Send + 'static,
    {
        let permit = self
            .query_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::unavailable("The index is busy; retry shortly."))?;

        let database_path = self.database_path.clone();
        let network = self.network.clone();

        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut reader = IndexReader::open(&database_path, &network)?;
            query(&mut reader)
        })
        .await
        .map_err(|error| {
            eprintln!("Index query task failed: {error}");
            ApiError::unavailable("Could not read the index; retry shortly.")
        })?;

        result.map_err(ApiError::from)
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Server metadata & health
        .route("/info", get(routes::server_info))
        .route("/ready", get(routes::readiness))
        // Chain & fee endpoints (served from truenorthd)
        .route("/v1/chain/tip", get(routes::chain_tip))
        .route("/v1/fees/estimate", get(routes::fee_estimate))
        .route("/v2/fees/estimate", get(routes::fee_estimate_v2))
        // Indexed wallet queries (served from local SQLite index)
        .route("/v1/addresses/activity", post(routes::address_activity))
        .route("/v1/utxos", post(routes::utxos))
        .route("/v1/transactions", post(routes::transactions))
        // Transaction validation, broadcast, and status
        .route(
            "/v1/transactions/validate",
            post(routes::validate_transaction),
        )
        .route(
            "/v1/transactions/broadcast",
            post(routes::broadcast_transaction),
        )
        .route("/v1/transactions/status", post(routes::transaction_status))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(middleware::map_response(error::format_response))
        .with_state(state)
}
