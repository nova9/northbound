//! Optional solo-mining HTTP API, independent of wallet state and index queries.
mod routes;
mod templates;

use std::{env, num::NonZeroU32, sync::Arc};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use tokio::sync::{Semaphore, watch};
use truenorth_rpc::{MAX_BLOCK_HEX_LENGTH, NodeClient};

use crate::{error, rate_limit};
use templates::CachedTemplate;

pub use templates::run_template_watcher;

const MAX_TEMPLATE_CLIENTS: usize = 256;
const MAX_HASH_REQUESTS: usize = 8;
const MAX_SUBMISSIONS: usize = 1;

pub struct MiningState {
    node: NodeClient,
    templates: watch::Sender<Option<Arc<CachedTemplate>>>,
    template_clients: Semaphore,
    hash_requests: Semaphore,
    submissions: Semaphore,
}

impl MiningState {
    pub fn new(node: NodeClient) -> Self {
        let (templates, _) = watch::channel(None);
        Self {
            node,
            templates,
            template_clients: Semaphore::new(MAX_TEMPLATE_CLIENTS),
            hash_requests: Semaphore::new(MAX_HASH_REQUESTS),
            submissions: Semaphore::new(MAX_SUBMISSIONS),
        }
    }
}

pub struct MiningConfig {
    pub enabled: bool,
    pub requests_per_second: NonZeroU32,
}

impl MiningConfig {
    pub fn from_environment() -> Result<Self, String> {
        let enabled = match env::var("TRUENORTH_MINING_ENABLED") {
            Err(env::VarError::NotPresent) => Ok(true),
            Ok(value) if value == "true" => Ok(true),
            Ok(value) if value == "false" => Ok(false),
            _ => Err("TRUENORTH_MINING_ENABLED must be 'true' or 'false'".to_owned()),
        }?;
        let requests_per_second = env::var("TRUENORTH_MINING_REQUESTS_PER_SECOND")
            .unwrap_or_else(|_| "100".into())
            .parse::<NonZeroU32>()
            .map_err(|_| {
                "TRUENORTH_MINING_REQUESTS_PER_SECOND must be a positive 32-bit integer".to_owned()
            })?;
        Ok(Self {
            enabled,
            requests_per_second,
        })
    }
}

pub fn build_router(state: Arc<MiningState>, requests_per_second: NonZeroU32) -> Router {
    let rate_limit = Arc::new(rate_limit::RateLimit::new(requests_per_second));
    let submissions = Router::new()
        .route("/blocks", post(routes::submit_block))
        // Acquire the submission slot before buffering a potentially large JSON body.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            routes::limit_submissions,
        ))
        .layer(DefaultBodyLimit::max(MAX_BLOCK_HEX_LENGTH + 1024));
    let routes = Router::new()
        .route("/template", get(routes::block_template))
        .route("/block-hash", get(routes::block_hash))
        .merge(submissions)
        .layer(middleware::from_fn_with_state(
            rate_limit,
            rate_limit::limit_request_rate,
        ))
        .with_state(state);
    Router::new()
        .nest("/v1/mining", routes)
        .layer(middleware::map_response(error::format_response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    fn state() -> Arc<MiningState> {
        Arc::new(MiningState::new(
            NodeClient::new("http://127.0.0.1:1", "unused-cookie".into(), "regtest").unwrap(),
        ))
    }

    fn request(method: &str, path: &str, body: String) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn malformed_inputs_are_rejected_before_contacting_the_node() {
        let router = build_router(state(), NonZeroU32::new(100).unwrap());
        for (method, path, body, expected) in [
            (
                "GET",
                "/v1/mining/block-hash?height=-1",
                String::new(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "GET",
                "/v1/mining/block-hash?height=2147483648",
                String::new(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "GET",
                "/v1/mining/block-hash?height=abc",
                String::new(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "GET",
                "/v1/mining/template?longpollid=",
                String::new(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "POST",
                "/v1/mining/blocks",
                json!({"block_hex": "zz"}).to_string(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "POST",
                "/v1/mining/blocks",
                "{".into(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "POST",
                "/v1/mining/blocks",
                json!({"block_hex": "00".repeat(MAX_BLOCK_HEX_LENGTH / 2 + 1024)}).to_string(),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
        ] {
            let response = router
                .clone()
                .oneshot(request(method, path, body))
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{path}");
            assert_eq!(response.headers()["cache-control"], "no-store");
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert!(serde_json::from_slice::<Value>(&body).unwrap()["message"].is_string());
        }
    }

    #[tokio::test]
    async fn mining_limits_are_independent_and_busy_submissions_are_not_buffered() {
        let state = state();
        let wallet = crate::build_router(
            Arc::new(crate::AppState::new(
                state.node.clone(),
                "unused.sqlite".into(),
                "regtest".into(),
            )),
            NonZeroU32::new(1).unwrap(),
        );
        let router = wallet.merge(build_router(state.clone(), NonZeroU32::new(1).unwrap()));
        let permit = state.submissions.acquire().await.unwrap();
        let response = router
            .clone()
            .oneshot(request("POST", "/v1/mining/blocks", "{".into()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        drop(permit);
        let response = router
            .clone()
            .oneshot(request("GET", "/v1/mining/template", String::new()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let response = router
            .oneshot(request("GET", "/info", String::new()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn exhausted_template_and_hash_capacity_returns_without_node_work() {
        let state = state();
        let router = build_router(state.clone(), NonZeroU32::new(100).unwrap());
        let template_slots = state
            .template_clients
            .acquire_many(MAX_TEMPLATE_CLIENTS as u32)
            .await
            .unwrap();
        let hash_slots = state
            .hash_requests
            .acquire_many(MAX_HASH_REQUESTS as u32)
            .await
            .unwrap();
        for path in ["/v1/mining/template", "/v1/mining/block-hash?height=0"] {
            let response = router
                .clone()
                .oneshot(request("GET", path, String::new()))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }
        drop(template_slots);
        drop(hash_slots);
        assert_eq!(
            state.template_clients.available_permits(),
            MAX_TEMPLATE_CLIENTS
        );
        assert_eq!(state.hash_requests.available_permits(), MAX_HASH_REQUESTS);
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_submission_times_out_and_releases_its_slot() {
        use std::time::Duration;
        let state = state();
        let router = Router::new()
            .route(
                "/blocks",
                post(|| async {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    StatusCode::OK
                }),
            )
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                routes::limit_submissions,
            ));
        let started = tokio::time::Instant::now();
        let response = router
            .oneshot(request("POST", "/blocks", String::new()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(started.elapsed(), Duration::from_secs(30));
        assert_eq!(state.submissions.available_permits(), MAX_SUBMISSIONS);
    }

    #[tokio::test]
    async fn daemon_submission_results_and_failures_remain_distinct() {
        use axum::{Json, extract::State};
        use tokio::sync::Mutex;
        let results = Arc::new(Mutex::new(std::collections::VecDeque::from([
            json!({"result": null, "error": null}),
            json!({"result": "duplicate", "error": null}),
            json!({"result": "bad-cb-amount", "error": null}),
            json!({"result": "duplicate-invalid", "error": null}),
            json!({"result": "inconclusive", "error": null}),
            json!({"result": "duplicate-inconclusive", "error": null}),
            json!({"result": null, "error": {"code": -22, "message": "Block decode failed"}}),
            json!({"result": null, "error": {"code": -1, "message": "Internal node error"}}),
            json!({"result": 42, "error": null}),
            json!({"result": null, "error": null}),
        ])));
        let rpc = Router::new()
            .route(
                "/",
                post(
                    |State(results): State<Arc<Mutex<std::collections::VecDeque<Value>>>>,
                     Json(request): Json<Value>| async move {
                        assert!(matches!(
                            request["method"].as_str(),
                            Some("submitblock" | "getblockchaininfo")
                        ));
                        let mut result = results.lock().await.pop_front().unwrap();
                        result["id"] = request["id"].clone();
                        result["jsonrpc"] = json!("2.0");
                        Json(result)
                    },
                ),
            )
            .with_state(results);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let rpc_task = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        let cookie = directory.path().join(".cookie");
        std::fs::write(&cookie, "test:password").unwrap();
        let state = Arc::new(MiningState::new(
            NodeClient::new(&url, cookie, "regtest").unwrap(),
        ));
        let node = state.node.clone();
        let router = build_router(state, NonZeroU32::new(100).unwrap());
        for (http_status, status, result) in [
            (200, "accepted", Value::Null),
            (200, "duplicate", json!("duplicate")),
            (200, "rejected", json!("bad-cb-amount")),
            (200, "rejected", json!("duplicate-invalid")),
            (200, "inconclusive", json!("inconclusive")),
            (200, "inconclusive", json!("duplicate-inconclusive")),
            (400, "", Value::Null),
            (502, "", Value::Null),
            (502, "", Value::Null),
        ] {
            let response = router
                .clone()
                .oneshot(request(
                    "POST",
                    "/v1/mining/blocks",
                    json!({"block_hex": "00".repeat(81)}).to_string(),
                ))
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), http_status);
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let value: Value = serde_json::from_slice(&body).unwrap();
            if http_status == 200 {
                assert_eq!(value, json!({"status": status, "result": result}));
            } else {
                assert!(value["message"].is_string());
            }
        }
        // Null is a success only for submitblock; existing RPC calls remain strict.
        assert!(matches!(
            node.fetch_tip().await,
            Err(truenorth_rpc::Error::InvalidData(_))
        ));
        rpc_task.abort();
    }
}
