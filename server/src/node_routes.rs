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
    let transaction_id = match state
        .node
        .broadcast_transaction(&request.transaction_hex)
        .await
    {
        Ok(transaction_id) => transaction_id,
        Err(error)
            if matches!(&error, truenorth_rpc::Error::Remote {
            method, code: -25, message,
        } if method == "sendrawtransaction" && message == "bad-txns-inputs-missingorspent") =>
        {
            // With every output spent, the daemon can report missing inputs even
            // for a confirmed transaction. Only the exact indexed ID proves success.
            let transaction_id = state
                .node
                .decode_transaction_id(&request.transaction_hex)
                .await?;
            let id = decode_hash(&transaction_id)?;
            let status = query_index(state.clone(), move |reader| {
                reader.read_snapshot(&[])?.query_transaction_status(&id)
            })
            .await?;
            if status.block_height.is_none() {
                return Err(ApiError::from_broadcast_error(error));
            }
            transaction_id
        }
        Err(error) => return Err(ApiError::from_broadcast_error(error)),
    };
    Ok(Json(BroadcastTransactionResponse { transaction_id }))
}

pub(crate) async fn transaction_status(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TransactionStatusRequest>,
) -> Result<Json<TransactionStatusResponse>, ApiError> {
    let id = decode_hash(&request.transaction_id)
        .map_err(|_| ApiError::bad_request("transaction ID must be 64 hexadecimal characters"))?;
    let status = query_index(state.clone(), move |reader| {
        reader.read_snapshot(&[])?.query_transaction_status(&id)
    })
    .await?;
    if status.known {
        return Ok(Json(status));
    }
    let status = state
        .node
        .fetch_transaction_status(&request.transaction_id)
        .await?;
    let (known, block_height) = match status {
        truenorth_rpc::TransactionStatus::Unknown => (false, None),
        truenorth_rpc::TransactionStatus::Pending => (true, None),
        truenorth_rpc::TransactionStatus::Confirmed { block_height } => (true, Some(block_height)),
    };
    Ok(Json(TransactionStatusResponse {
        known,
        block_height,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::post};
    use serde_json::{Value, json};
    use std::num::NonZeroU32;
    use tower::ServiceExt;

    use crate::test_support::{RpcFixture, rejection, request, result, tip};

    #[tokio::test]
    async fn non_json_http_failures_remain_retryable_and_return_502() {
        for http_status in [401, 503] {
            for body in [
                "",
                "Service unavailable",
                "<html>Service unavailable</html>",
            ] {
                let fixture =
                    RpcFixture::new_raw("regtest", move |_| (http_status, body.into())).await;
                let error = fixture.node.fetch_tip().await.unwrap_err();
                assert!(
                    matches!(&error, truenorth_rpc::Error::Transport(_)),
                    "{error}"
                );
                assert!(matches!(
                    indexer::Error::from(error),
                    indexer::Error::Rpc(_)
                ));
                let (status, response) =
                    request(fixture.router("regtest"), "/v1/chain/tip", None).await;
                assert_eq!(status, 502);
                assert_eq!(
                    response["message"],
                    "The blockchain node is unavailable or returned an invalid response."
                );
            }
        }
        let fixture = RpcFixture::new_raw("regtest", |_| (200, "invalid JSON".into())).await;
        assert!(matches!(
            fixture.node.fetch_tip().await,
            Err(truenorth_rpc::Error::InvalidData(_))
        ));
    }

    #[tokio::test]
    async fn rpc_errors_preserve_codes_and_reasons_on_success_and_failure_http_statuses() {
        for http_status in [200, 500] {
            let fixture =
                RpcFixture::new("regtest", move |_| (http_status, rejection(-26, "dust"))).await;
            let error = fixture.node.fetch_tip().await.unwrap_err();
            assert!(matches!(error, truenorth_rpc::Error::Remote {
                method, code: -26, message,
            } if method == "getblockchaininfo" && message == "dust"));
        }
        for body in [
            json!("not an RPC response"),
            json!({"jsonrpc": "2.0", "id": "northbound", "error": {"code": "-26", "message": "dust"}}),
            json!({"jsonrpc": "2.0", "id": "northbound", "error": {"code": -26}}),
            json!({"jsonrpc": "2.0", "id": "wrong", "error": {"code": -26, "message": "dust"}}),
        ] {
            let fixture = RpcFixture::new("regtest", move |_| (500, body.clone())).await;
            assert!(matches!(
                fixture.node.fetch_tip().await,
                Err(truenorth_rpc::Error::InvalidData(_))
            ));
        }
    }

    #[tokio::test]
    async fn broadcast_returns_rejection_reasons_and_keeps_upstream_failures_private() {
        for (code, reason, expected) in [
            (-22, "TX decode failed", 400),
            (-26, "dust", 400),
            (-26, "txn-mempool-conflict", 409),
            (-25, "bad-txns-inputs-missingorspent", 409),
            (-28, "internal infrastructure detail", 502),
        ] {
            let fixture = RpcFixture::new("regtest", move |rpc| {
                match rpc["method"].as_str().unwrap() {
                    "getblockchaininfo" => (200, tip("regtest")),
                    "decoderawtransaction" => (200, result(json!({"txid": "03".repeat(32)}))),
                    "sendrawtransaction" => (500, rejection(code, reason)),
                    method => panic!("unexpected method: {method}"),
                }
            })
            .await;
            let _database = fixture.create_ready_index();
            let (status, body) = request(
                fixture.router("regtest"),
                "/v1/transactions/broadcast",
                Some(json!({"transaction_hex": "00"})),
            )
            .await;
            assert_eq!(status, expected);
            if expected == 502 {
                assert!(!body.to_string().contains(reason));
            } else {
                assert_eq!(body["message"], reason);
            }
        }
        let fixture = RpcFixture::new("regtest", |rpc| {
            if rpc["method"] == "getblockchaininfo" {
                return (200, tip("regtest"));
            }
            (200, result(json!("invalid transaction ID")))
        })
        .await;
        let (status, _) = request(
            fixture.router("regtest"),
            "/v1/transactions/broadcast",
            Some(json!({"transaction_hex": "00"})),
        )
        .await;
        assert_eq!(status, 502);
        let router = fixture.router("regtest");
        fixture.task_abort();
        assert!(matches!(
            fixture.node.fetch_tip().await,
            Err(truenorth_rpc::Error::Transport(_))
        ));
        let (status, _) = request(
            router,
            "/v1/transactions/broadcast",
            Some(json!({"transaction_hex": "00"})),
        )
        .await;
        assert_eq!(status, 502);
    }

    #[tokio::test]
    async fn chain_tip_returns_only_height_and_rejects_a_network_mismatch() {
        for (configured, actual, expected) in [
            ("main", "main", 200),
            ("regtest", "regtest", 200),
            ("regtest", "main", 502),
        ] {
            let fixture = RpcFixture::new(configured, move |_| (200, tip(actual))).await;
            let (status, body) = request(fixture.router(configured), "/v1/chain/tip", None).await;
            assert_eq!(status, expected);
            if status == 200 {
                assert_eq!(body, json!({"height": 12}));
            }
        }
    }

    #[tokio::test]
    async fn status_fallback_distinguishes_unknown_pending_confirmed_and_unavailable() {
        for (raw, header, expected_status, known, height) in [
            (
                rejection(
                    -5,
                    "No such mempool or blockchain transaction. Use gettransaction for wallet transactions.",
                ),
                json!(null),
                200,
                false,
                None,
            ),
            (
                result(json!({"txid": "03".repeat(32)})),
                json!(null),
                200,
                true,
                None,
            ),
            (
                result(
                    json!({"txid": "03".repeat(32), "blockhash": "01".repeat(32), "confirmations": 2}),
                ),
                result(json!({"hash": "01".repeat(32), "confirmations": 2, "height": 12})),
                200,
                true,
                Some(12),
            ),
            (
                rejection(
                    -5,
                    "No such mempool transaction. Use -txindex or provide a block hash to enable blockchain transaction queries.",
                ),
                json!(null),
                502,
                false,
                None,
            ),
            (
                result(json!({"txid": "04".repeat(32)})),
                json!(null),
                502,
                false,
                None,
            ),
            (
                result(
                    json!({"txid": "03".repeat(32), "blockhash": "01".repeat(32), "confirmations": 1}),
                ),
                result(json!({"hash": "01".repeat(32), "confirmations": 1, "height": -1})),
                502,
                false,
                None,
            ),
            (
                result(
                    json!({"txid": "03".repeat(32), "blockhash": "01".repeat(32), "confirmations": 0}),
                ),
                json!(null),
                502,
                false,
                None,
            ),
        ] {
            let fixture = RpcFixture::new("regtest", move |rpc| {
                assert_eq!(
                    rpc["params"][0],
                    if rpc["method"] == "getrawtransaction" {
                        "03".repeat(32)
                    } else {
                        "01".repeat(32)
                    }
                );
                match rpc["method"].as_str().unwrap() {
                    "getrawtransaction" => (200, raw.clone()),
                    "getblockheader" => (200, header.clone()),
                    method => panic!("unexpected method: {method}"),
                }
            })
            .await;
            let _database = fixture.create_ready_index();
            let router = fixture.router("regtest");
            let (status, body) = request(
                router.clone(),
                "/v1/transactions/status",
                Some(json!({"transaction_id": "03".repeat(32)})),
            )
            .await;
            assert_eq!(status, expected_status, "{body}");
            if status == 200 {
                assert_eq!(body, json!({"known": known, "block_height": height}));
            }
            let (status, body) = request(
                router,
                "/v1/transactions/status",
                Some(json!({"transaction_id": "02".repeat(32)})),
            )
            .await;
            assert_eq!(status, 200);
            assert_eq!(body, json!({"known": true, "block_height": 0}));
        }
    }

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
