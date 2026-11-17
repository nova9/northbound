use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Query, Request, State},
    middleware::Next,
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use truenorth_rpc::{Error, validate_block_hex};

use super::{MiningState, templates::wait_for_template};
use crate::error::ApiError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TemplateQuery {
    longpollid: Option<String>,
}

pub(super) async fn block_template(
    State(state): State<Arc<MiningState>>,
    Query(query): Query<TemplateQuery>,
) -> Result<Response, ApiError> {
    if let Some(id) = &query.longpollid {
        if id.is_empty() || id.len() > 128 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(ApiError::bad_request(
                "longpollid must contain 1 to 128 ASCII letters or digits.",
            ));
        }
    }
    let _permit = state
        .template_clients
        .try_acquire()
        .map_err(|_| ApiError::unavailable("Too many mining clients; retry shortly."))?;
    wait_for_template(&state, query.longpollid.as_deref()).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HashQuery {
    height: u32,
}

pub(super) async fn block_hash(
    State(state): State<Arc<MiningState>>,
    Query(query): Query<HashQuery>,
) -> Result<Json<Value>, ApiError> {
    // Core heights are signed 32-bit integers.
    if query.height > i32::MAX as u32 {
        return Err(ApiError::bad_request("height exceeds the supported range."));
    }
    let _permit = state
        .hash_requests
        .try_acquire()
        .map_err(|_| ApiError::unavailable("Mining block lookups are busy; retry shortly."))?;
    let hash = match state.node.fetch_block_hash(i64::from(query.height)).await {
        Err(Error::Remote { code: -8, .. }) => {
            return Err(ApiError::bad_request("Block height is not available."));
        }
        result => result?,
    };
    // Hash bytes use the same display order as the daemon's hexadecimal hash.
    let mut encoded = String::with_capacity(64);
    for byte in hash {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}")
            .map_err(|_| ApiError::unavailable("Could not encode block hash."))?;
    }
    Ok(Json(json!({"height": query.height, "hash": encoded})))
}

pub(super) async fn limit_submissions(
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
    tokio::time::timeout(Duration::from_secs(30), next.run(request))
        .await
        .map_err(|_| {
            ApiError::unavailable("Block submission timed out; check block status before retrying.")
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BlockRequest {
    block_hex: String,
}

pub(super) async fn submit_block(
    State(state): State<Arc<MiningState>>,
    Json(request): Json<BlockRequest>,
) -> Result<Json<Value>, ApiError> {
    validate_block_hex(&request.block_hex)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let result = match state.node.submit_block(&request.block_hex).await {
        Err(Error::Remote { code: -22, .. }) => {
            return Err(ApiError::bad_request(
                "The node could not decode the submitted block.",
            ));
        }
        result => result?,
    };
    let status = match result.as_deref() {
        None => "accepted",
        Some("duplicate") => "duplicate",
        Some("inconclusive" | "duplicate-inconclusive") => "inconclusive",
        Some(_) => "rejected",
    };
    Ok(Json(json!({"status": status, "result": result})))
}
