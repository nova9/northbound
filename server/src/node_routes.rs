use std::sync::Arc;

use axum::{Json, extract::State};
use shared::{
    BroadcastTransactionResponse, ChainTip, FeeEstimate, FeeEstimateV2, SignedTransactionRequest,
    TransactionStatusRequest, TransactionStatusResponse, TransactionValidation,
};
use truenorth_rpc::{decode_hash, validate_transaction_hex};

use crate::{AppState, api::query_index, error::ApiError};

pub(crate) async fn chain_tip(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ChainTip>, ApiError> {
    let tip = state.node.fetch_tip().await?;
    Ok(Json(ChainTip {
        height: tip.blocks as u64,
    }))
}

pub(crate) async fn fee_estimate(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FeeEstimate>, ApiError> {
    let estimate = state.node.estimate_fee_rate().await?;
    Ok(Json(FeeEstimate {
        atomic_units_per_vbyte: estimate.atomic_units_per_kvbyte.div_ceil(1000),
        target_blocks: estimate.target_blocks,
    }))
}

pub(crate) async fn fee_estimate_v2(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FeeEstimateV2>, ApiError> {
    let estimate = state.node.estimate_fee_rate().await?;
    Ok(Json(FeeEstimateV2 {
        atomic_units_per_kvbyte: estimate.atomic_units_per_kvbyte,
        target_blocks: estimate.target_blocks,
    }))
}

pub(crate) async fn validate_transaction(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SignedTransactionRequest>,
) -> Result<Json<TransactionValidation>, ApiError> {
    validate_transaction_hex(&request.transaction_hex)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let validation = state
        .node
        .test_transaction_acceptance(&request.transaction_hex)
        .await?;
    Ok(Json(TransactionValidation {
        allowed: validation.allowed,
        rejection_reason: validation.rejection_reason,
    }))
}

pub(crate) async fn broadcast_transaction(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SignedTransactionRequest>,
) -> Result<Json<BroadcastTransactionResponse>, ApiError> {
    validate_transaction_hex(&request.transaction_hex)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let transaction_id = state
        .node
        .broadcast_transaction(&request.transaction_hex)
        .await?;
    Ok(Json(BroadcastTransactionResponse { transaction_id }))
}

pub(crate) async fn transaction_status(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TransactionStatusRequest>,
) -> Result<Json<TransactionStatusResponse>, ApiError> {
    let id = decode_hash(&request.transaction_id)
        .map_err(|_| ApiError::bad_request("transaction ID must be 64 hexadecimal characters"))?;
    let known = query_index(state.clone(), move |reader| {
        reader.read_snapshot(&[])?.query_transaction_known(&id)
    })
    .await?;
    let known = if known {
        true
    } else {
        state
            .node
            .is_transaction_known(&request.transaction_id)
            .await?
    };
    Ok(Json(TransactionStatusResponse { known }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::post};
    use serde_json::{Value, json};
    use std::num::NonZeroU32;
    use tower::ServiceExt;

    #[tokio::test]
    async fn fee_endpoints_preserve_v1_and_return_precise_kvbytes_in_v2() {
        // A controlled RPC rate just above 1 unit/vB exposes premature rounding.
        let rpc = Router::new().route(
            "/",
            post(|Json(request): Json<Value>| async move {
                let result = match request["method"].as_str().unwrap() {
                    "estimatesmartfee" => json!({"feerate": 0.00001001, "blocks": 6}),
                    "getmempoolinfo" => json!({"mempoolminfee": 0.00001000}),
                    "getnetworkinfo" => json!({"relayfee": 0.00001000}),
                    method => panic!("unexpected RPC method: {method}"),
                };
                Json(
                    json!({"jsonrpc": "2.0", "result": result, "error": null, "id": request["id"]}),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let rpc_task = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        let cookie = directory.path().join(".cookie");
        std::fs::write(&cookie, "test:password").unwrap();
        let node = truenorth_rpc::NodeClient::new(&url, cookie, "regtest").unwrap();
        let state = Arc::new(AppState::new(
            node,
            directory.path().join("chain.sqlite"),
            "regtest".into(),
        ));
        let router = crate::build_router(state, NonZeroU32::new(100).unwrap());
        for (path, expected) in [
            (
                "/v1/fees/estimate",
                json!({"atomic_units_per_vbyte": 2, "target_blocks": 6}),
            ),
            (
                "/v2/fees/estimate",
                json!({"atomic_units_per_kvbyte": 1001, "target_blocks": 6}),
            ),
        ] {
            let response = router
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), expected);
        }
        rpc_task.abort();
    }
}
