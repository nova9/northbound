use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use shared::ApiErrorResponse;

pub(crate) async fn format_response(mut response: Response) -> Response {
    let status = response.status();
    let is_json = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes().starts_with(b"application/json"));
    if (status.is_client_error() || status.is_server_error()) && !is_json {
        let message = match status {
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                "Request must contain valid JSON with the required fields."
            }
            StatusCode::UNSUPPORTED_MEDIA_TYPE => "Request must use application/json.",
            StatusCode::PAYLOAD_TOO_LARGE => "Request exceeds the 1 MB body limit.",
            _ => "Request could not be served.",
        };
        response = (
            status,
            Json(ApiErrorResponse {
                message: message.into(),
            }),
        )
            .into_response();
    }
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

pub(crate) struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorResponse {
                message: self.message,
            }),
        )
            .into_response()
    }
}

impl From<indexer::Error> for ApiError {
    fn from(error: indexer::Error) -> Self {
        match error {
            indexer::Error::InvalidRequest(message) => Self::bad_request(message),
            indexer::Error::Unavailable(message) => Self::unavailable(message),
            indexer::Error::CursorChanged => Self {
                status: StatusCode::CONFLICT,
                message: "The index changed; restart history pagination.".into(),
            },
            error => {
                eprintln!("Index query failed: {error}");
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: "Could not read wallet data.".into(),
                }
            }
        }
    }
}

impl From<truenorth_rpc::Error> for ApiError {
    fn from(error: truenorth_rpc::Error) -> Self {
        eprintln!("Node request failed: {error}");
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: "The blockchain node is unavailable or returned an invalid response.".into(),
        }
    }
}
