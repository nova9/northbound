//! Optional solo-mining HTTP API, kept completely separate from wallet state
//! and index queries so it can be enabled or removed independently.

mod templates;

use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Query, Request, State},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Semaphore, watch};
use truenorth_rpc::{Error, MAX_BLOCK_HEX_LENGTH, NodeClient, validate_block_hex};

use crate::{error, error::ApiError};
use templates::{CachedTemplate, wait_for_template};

pub use templates::run_template_watcher;

const MAX_TEMPLATE_CLIENTS: usize = 256;
const MAX_HASH_REQUESTS: usize = 8;
const MAX_SUBMISSIONS: usize = 1;
const MAX_LONGPOLL_ID_LENGTH: usize = 128;
const SUBMISSION_TIMEOUT: Duration = Duration::from_secs(30);
const SUBMISSION_JSON_OVERHEAD_BYTES: usize = 1024;

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

pub fn build_router(state: Arc<MiningState>) -> Router {
    // Block submission gets its own concurrency gate and body limit so a busy
    // or slow upload is rejected before buffering megabytes of JSON.
    let submission_routes = Router::new()
        .route("/blocks", post(submit_block))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            limit_submissions,
        ))
        .layer(DefaultBodyLimit::max(
            MAX_BLOCK_HEX_LENGTH + SUBMISSION_JSON_OVERHEAD_BYTES,
        ));

    let mining_routes = Router::new()
        .route("/template", get(block_template))
        .route("/block-hash", get(block_hash))
        .merge(submission_routes)
        .with_state(state);

    Router::new()
        .nest("/v1/mining", mining_routes)
        .layer(middleware::map_response(error::format_response))
}

// --- Handlers ---

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateQuery {
    longpollid: Option<String>,
}

async fn block_template(
    State(state): State<Arc<MiningState>>,
    Query(query): Query<TemplateQuery>,
) -> Result<Response, ApiError> {
    if let Some(id) = &query.longpollid {
        validate_longpoll_id(id)?;
    }

    let _permit = state
        .template_clients
        .try_acquire()
        .map_err(|_| ApiError::unavailable("Too many mining clients; retry shortly."))?;

    wait_for_template(&state, query.longpollid.as_deref()).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HashQuery {
    height: u32,
}

async fn block_hash(
    State(state): State<Arc<MiningState>>,
    Query(query): Query<HashQuery>,
) -> Result<Json<Value>, ApiError> {
    // TrueNorth Core block heights are signed 32-bit integers.
    if query.height > i32::MAX as u32 {
        return Err(ApiError::bad_request("height exceeds the supported range."));
    }

    let _permit = state
        .hash_requests
        .try_acquire()
        .map_err(|_| ApiError::unavailable("Mining block lookups are busy; retry shortly."))?;

    let hash = match state.node.fetch_block_hash(i64::from(query.height)).await {
        Ok(hash) => hash,
        Err(Error::Remote { code: -8, .. }) => {
            return Err(ApiError::bad_request("Block height is not available."));
        }
        Err(error) => return Err(error.into()),
    };

    Ok(Json(json!({
        "height": query.height,
        "hash": encode_hex(&hash),
    })))
}

async fn limit_submissions(
    State(state): State<Arc<MiningState>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let _permit = state
        .submissions
        .try_acquire()
        .map_err(|_| ApiError::unavailable("Block submission is busy; retry shortly."))?;

    // Bound slow uploads as well as node validation so one client cannot hold
    // the only submission slot indefinitely.
    tokio::time::timeout(SUBMISSION_TIMEOUT, next.run(request))
        .await
        .map_err(|_| {
            ApiError::unavailable("Block submission timed out; check block status before retrying.")
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockRequest {
    block_hex: String,
}

async fn submit_block(
    State(state): State<Arc<MiningState>>,
    Json(request): Json<BlockRequest>,
) -> Result<Json<Value>, ApiError> {
    validate_block_hex(&request.block_hex)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;

    let result = match state.node.submit_block(&request.block_hex).await {
        Ok(result) => result,
        Err(Error::Remote { code: -22, .. }) => {
            return Err(ApiError::bad_request(
                "The node could not decode the submitted block.",
            ));
        }
        Err(error) => return Err(error.into()),
    };

    let status = classify_submit_result(result.as_deref());
    Ok(Json(json!({"status": status, "result": result})))
}

fn validate_longpoll_id(id: &str) -> Result<(), ApiError> {
    let is_valid = !id.is_empty()
        && id.len() <= MAX_LONGPOLL_ID_LENGTH
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric());

    if !is_valid {
        return Err(ApiError::bad_request(
            "longpollid must contain 1 to 128 ASCII letters or digits.",
        ));
    }

    Ok(())
}

fn classify_submit_result(result: Option<&str>) -> &'static str {
    match result {
        None => "accepted",
        Some("duplicate") => "duplicate",
        Some("inconclusive" | "duplicate-inconclusive") => "inconclusive",
        Some(_) => "rejected",
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::test_support::RpcFixture;

    fn offline_state() -> Arc<MiningState> {
        let node =
            NodeClient::new("http://127.0.0.1:1", "unused-cookie".into(), "regtest").unwrap();
        Arc::new(MiningState::new(node))
    }

    fn json_request(method: &str, path: &str, body: String) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn malformed_inputs_are_rejected_before_contacting_the_node() {
        let router = build_router(offline_state());
        let cases = [
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
        ];

        for (method, path, body, expected_status) in cases {
            let response = router
                .clone()
                .oneshot(json_request(method, path, body))
                .await
                .unwrap();

            assert_eq!(response.status(), expected_status, "{path}");
            assert_eq!(response.headers()["cache-control"], "no-store");

            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert!(serde_json::from_slice::<Value>(&body).unwrap()["message"].is_string());
        }
    }

    #[tokio::test]
    async fn busy_submissions_are_rejected_without_buffering_the_body() {
        let mining_state = offline_state();
        let router = build_router(mining_state.clone());

        // A busy submission returns 503 before parsing the invalid JSON body.
        let permit = mining_state.submissions.acquire().await.unwrap();
        let response = router
            .oneshot(json_request("POST", "/v1/mining/blocks", "{".into()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        drop(permit);
    }

    #[tokio::test]
    async fn exhausted_template_and_hash_capacity_returns_without_node_work() {
        let state = offline_state();
        let router = build_router(state.clone());

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
                .oneshot(json_request("GET", path, String::new()))
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
        let state = offline_state();
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
                limit_submissions,
            ));

        let started = tokio::time::Instant::now();
        let response = router
            .oneshot(json_request("POST", "/blocks", String::new()))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(started.elapsed(), SUBMISSION_TIMEOUT);
        assert_eq!(state.submissions.available_permits(), MAX_SUBMISSIONS);
    }

    #[tokio::test]
    async fn daemon_submission_results_and_failures_remain_distinct() {
        let replies = Arc::new(Mutex::new(VecDeque::from([
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

        let fixture = RpcFixture::new("regtest", move |rpc| {
            assert!(matches!(
                rpc["method"].as_str(),
                Some("submitblock" | "getblockchaininfo")
            ));
            let mut reply = replies.lock().unwrap().pop_front().unwrap();
            reply["id"] = rpc["id"].clone();
            reply["jsonrpc"] = json!("2.0");
            (200, reply)
        })
        .await;

        let state = Arc::new(MiningState::new(fixture.node.clone()));
        let router = build_router(state);

        let expected_outcomes = [
            (200, "accepted", Value::Null),
            (200, "duplicate", json!("duplicate")),
            (200, "rejected", json!("bad-cb-amount")),
            (200, "rejected", json!("duplicate-invalid")),
            (200, "inconclusive", json!("inconclusive")),
            (200, "inconclusive", json!("duplicate-inconclusive")),
            (400, "", Value::Null),
            (502, "", Value::Null),
            (502, "", Value::Null),
        ];

        for (expected_http, expected_status, expected_result) in expected_outcomes {
            let response = router
                .clone()
                .oneshot(json_request(
                    "POST",
                    "/v1/mining/blocks",
                    json!({"block_hex": "00".repeat(81)}).to_string(),
                ))
                .await
                .unwrap();

            assert_eq!(response.status().as_u16(), expected_http);
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let value: Value = serde_json::from_slice(&body).unwrap();

            if expected_http == 200 {
                assert_eq!(
                    value,
                    json!({"status": expected_status, "result": expected_result})
                );
            } else {
                assert!(value["message"].is_string());
            }
        }

        // A JSON `null` result is only valid for `submitblock`; other RPC calls remain strict.
        assert!(matches!(
            fixture.node.fetch_tip().await,
            Err(truenorth_rpc::Error::InvalidData(_))
        ));
    }
}
