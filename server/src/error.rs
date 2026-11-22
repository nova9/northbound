use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use shared::ApiErrorResponse;

/// Ensures every error response uses our JSON `{"message": "..."}` format
/// (including Axum's own 400/413/415/422 rejections) and disables HTTP caching.
pub(crate) async fn format_response(mut response: Response) -> Response {
    let status = response.status();
    let is_error = status.is_client_error() || status.is_server_error();
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes().starts_with(b"application/json"));

    if is_error && !is_json {
        let message = match status {
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                "Request must contain valid JSON with the required fields."
            }
            StatusCode::UNSUPPORTED_MEDIA_TYPE => "Request must use application/json.",
            StatusCode::PAYLOAD_TOO_LARGE => "Request exceeds the configured body limit.",
            _ => "Request could not be served.",
        };
        response = json_error_response(status, message);
    }

    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
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

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
        }
    }

    /// Maps `sendrawtransaction` rejections to client-visible 400/409 errors.
    /// Any other RPC failure stays a generic 502 so node internals stay private.
    pub fn from_broadcast_error(error: truenorth_rpc::Error) -> Self {
        let truenorth_rpc::Error::Remote {
            method,
            code,
            message,
        } = &error
        else {
            return Self::from(error);
        };

        if method != "sendrawtransaction" {
            return Self::from(error);
        }

        // TrueNorth Core RPC error codes for sendrawtransaction:
        // -22: transaction decode failed
        // -25: missing/invalid inputs
        // -26: rejected by mempool policy
        // -27: transaction already in chain
        let status = match code {
            -25 | -26
                if message == "bad-txns-inputs-missingorspent"
                    || message == "txn-mempool-conflict" =>
            {
                StatusCode::CONFLICT
            }
            -22 | -25 | -26 => StatusCode::BAD_REQUEST,
            -27 => StatusCode::CONFLICT,
            _ => return Self::from(error),
        };

        Self {
            status,
            message: message.clone(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        json_error_response(self.status, self.message)
    }
}

impl From<indexer::Error> for ApiError {
    fn from(error: indexer::Error) -> Self {
        match error {
            indexer::Error::InvalidRequest(message) => Self::bad_request(message),
            indexer::Error::Unavailable(message) => Self::unavailable(message),
            indexer::Error::CursorChanged => {
                Self::conflict("The index changed; restart pagination.")
            }
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

fn json_error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ApiErrorResponse {
            message: message.into(),
        }),
    )
        .into_response()
}
