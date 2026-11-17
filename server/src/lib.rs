mod api;
pub mod config;
mod error;
pub mod mining;
mod node_routes;
mod rate_limit;
#[cfg(test)]
mod test_support;

use std::num::NonZeroU32;
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

pub fn build_router(state: Arc<AppState>, requests_per_second: NonZeroU32) -> Router {
    let rate_limit = Arc::new(rate_limit::RateLimit::new(requests_per_second));
    Router::new()
        .route("/info", get(api::server_info))
        .route("/v1/chain/tip", get(node_routes::chain_tip))
        .route("/v1/fees/estimate", get(node_routes::fee_estimate))
        .route("/v2/fees/estimate", get(node_routes::fee_estimate_v2))
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
        .layer(axum::middleware::from_fn_with_state(
            rate_limit,
            rate_limit::limit_request_rate,
        ))
        // Health probes must remain available when the API reaches its rate limit.
        .route("/ready", get(api::readiness))
        .layer(axum::middleware::map_response(error::format_response))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, http::StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn info_returns_version_and_network_without_a_node_or_index() {
        for network in ["main", "regtest"] {
            let directory = tempfile::tempdir().unwrap();
            let node = NodeClient::new(
                "http://127.0.0.1:1",
                directory.path().join("missing-cookie"),
                network,
            )
            .unwrap();
            let state = Arc::new(AppState::new(
                node,
                directory.path().join("missing-index.sqlite"),
                network.into(),
            ));
            let router = build_router(state, NonZeroU32::new(100).unwrap());

            let response = router
                .oneshot(Request::get("/info").body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "application/json");
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                info,
                serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "network": network})
            );
        }
    }
}
