use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use shared::ApiErrorResponse;
use tokio::sync::Mutex;

pub(crate) struct RateLimit {
    requests_per_second: NonZeroU32,
    window: Mutex<RequestWindow>,
}

struct RequestWindow {
    started_at: Instant,
    accepted_requests: u32,
}

impl RateLimit {
    pub(crate) fn new(requests_per_second: NonZeroU32) -> Self {
        Self {
            requests_per_second,
            window: Mutex::new(RequestWindow {
                started_at: Instant::now(),
                accepted_requests: 0,
            }),
        }
    }
}

impl RequestWindow {
    fn accept_request(&mut self, now: Instant, requests_per_second: NonZeroU32) -> bool {
        if now.duration_since(self.started_at) >= Duration::from_secs(1) {
            self.started_at = now;
            self.accepted_requests = 0;
        }
        if self.accepted_requests >= requests_per_second.get() {
            return false;
        }
        self.accepted_requests += 1;
        true
    }
}

pub(crate) async fn limit_request_rate(
    State(limit): State<Arc<RateLimit>>,
    request: Request,
    next: Next,
) -> Response {
    // Release the lock before reading the body or running database/RPC work.
    let accepted = {
        let mut window = limit.window.lock().await;
        window.accept_request(Instant::now(), limit.requests_per_second)
    };
    if !accepted {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "1")],
            Json(ApiErrorResponse {
                message: "The server request rate limit was reached; retry in one second.".into(),
            }),
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request};
    use tower::ServiceExt;

    #[test]
    fn requests_resume_when_the_one_second_window_expires() {
        let start = Instant::now();
        let limit = NonZeroU32::new(2).unwrap();
        let mut window = RequestWindow {
            started_at: start,
            accepted_requests: 0,
        };
        assert!(window.accept_request(start, limit));
        assert!(window.accept_request(start, limit));
        assert!(!window.accept_request(start + Duration::from_millis(999), limit));
        assert!(window.accept_request(start + Duration::from_secs(1), limit));
        assert!(window.accept_request(start + Duration::from_secs(1), limit));
        assert!(!window.accept_request(start + Duration::from_secs(1), limit));
        assert!(window.accept_request(start + Duration::from_secs(10), limit));
    }

    fn test_router(limit: u32) -> Router {
        let node = truenorth_rpc::NodeClient::new(
            "http://127.0.0.1:18443",
            "unused-cookie".into(),
            "regtest",
        )
        .unwrap();
        let state = Arc::new(crate::AppState::new(
            node,
            "missing-index.sqlite".into(),
            "regtest".into(),
        ));
        crate::build_router(state, NonZeroU32::new(limit).unwrap())
    }

    #[tokio::test]
    async fn endpoints_share_the_limit_and_return_retryable_json_errors() {
        let router = test_router(1);
        // Invalid bodies still count; a rejected request must not reach the node.
        let response = router
            .clone()
            .oneshot(Request::post("/v1/utxos").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let response = router
            .oneshot(Request::get("/v1/chain/tip").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "1");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let error: ApiErrorResponse = serde_json::from_slice(&body).unwrap();
        assert!(error.message.contains("retry in one second"));
    }

    #[tokio::test]
    async fn readiness_does_not_consume_the_api_budget_or_get_throttled() {
        let router = test_router(1);
        for (method, path, status) in [
            ("GET", "/ready", StatusCode::SERVICE_UNAVAILABLE),
            ("POST", "/v1/utxos", StatusCode::UNSUPPORTED_MEDIA_TYPE),
            ("GET", "/ready", StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
    }

    #[tokio::test]
    async fn concurrent_clients_cannot_exceed_the_shared_budget() {
        let router = test_router(4);
        let mut requests = tokio::task::JoinSet::new();
        for _ in 0..32 {
            let router = router.clone();
            requests.spawn(async move {
                router
                    .oneshot(Request::post("/v1/utxos").body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status()
            });
        }
        let mut accepted = 0;
        let mut rejected = 0;
        while let Some(result) = requests.join_next().await {
            match result.unwrap() {
                StatusCode::UNSUPPORTED_MEDIA_TYPE => accepted += 1,
                StatusCode::TOO_MANY_REQUESTS => rejected += 1,
                status => panic!("unexpected response: {status}"),
            }
        }
        assert_eq!(accepted, 4);
        assert_eq!(rejected, 28);
    }
}
