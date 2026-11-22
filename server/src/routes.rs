use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use indexer::decode_addresses;
use shared::{
    AddressActivityRequest, AddressActivityResponse, BroadcastTransactionResponse, ChainTip,
    DEFAULT_UTXO_PAGE_SIZE, FeeEstimate, FeeEstimateV2, IndexStatus, ServerInfo,
    SignedTransactionRequest, TransactionHistory, TransactionHistoryRequest,
    TransactionStatusRequest, TransactionStatusResponse, TransactionValidation, UtxoRequest,
    UtxoResponse,
};
use truenorth_rpc::{decode_hash, validate_transaction_hex};

use crate::{AppState, error::ApiError};

const DEFAULT_HISTORY_PAGE_SIZE: u16 = 100;

// --- Server metadata & readiness ---

pub(crate) async fn server_info(State(state): State<Arc<AppState>>) -> Json<ServerInfo> {
    Json(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        network: state.network.clone(),
    })
}

pub(crate) async fn readiness(
    State(state): State<Arc<AppState>>,
) -> Result<(StatusCode, Json<IndexStatus>), ApiError> {
    let status = state.read_index(|reader| reader.query_status()).await?;
    let http_status = if status.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    Ok((http_status, Json(status)))
}

// --- Chain & fee endpoints (served from truenorthd) ---

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

// --- Indexed wallet queries (served from local SQLite index) ---

pub(crate) async fn address_activity(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AddressActivityRequest>,
) -> Result<Json<AddressActivityResponse>, ApiError> {
    let scripts = decode_addresses(&request.addresses, &state.network)?;

    let response = state
        .read_index(move |reader| {
            let snapshot = reader.read_snapshot(&scripts)?;
            Ok(AddressActivityResponse {
                used_addresses: snapshot.query_script_activity()?,
                index: snapshot.status.clone(),
            })
        })
        .await?;

    Ok(Json(response))
}

pub(crate) async fn utxos(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UtxoRequest>,
) -> Result<Json<UtxoResponse>, ApiError> {
    let scripts = decode_addresses(&request.addresses, &state.network)?;
    let limit = request.limit.unwrap_or(DEFAULT_UTXO_PAGE_SIZE);

    let response = state
        .read_index(move |reader| {
            let snapshot = reader.read_snapshot(&scripts)?;
            snapshot.query_utxos(request.cursor.as_deref(), limit)
        })
        .await?;

    Ok(Json(response))
}

pub(crate) async fn transactions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TransactionHistoryRequest>,
) -> Result<Json<TransactionHistory>, ApiError> {
    let scripts = decode_addresses(&request.addresses, &state.network)?;
    let limit = request.limit.unwrap_or(DEFAULT_HISTORY_PAGE_SIZE);

    let response = state
        .read_index(move |reader| {
            let snapshot = reader.read_snapshot(&scripts)?;
            snapshot.query_transaction_history(request.cursor.as_deref(), limit)
        })
        .await?;

    Ok(Json(response))
}

// --- Transaction validation, broadcast, and status ---

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

    match state
        .node
        .broadcast_transaction(&request.transaction_hex)
        .await
    {
        Ok(transaction_id) => Ok(Json(BroadcastTransactionResponse { transaction_id })),
        Err(error) if is_missing_inputs_rejection(&error) => {
            // If all outputs of an already-confirmed transaction were later spent,
            // truenorthd reports missing inputs instead of "already in chain".
            // Treat a re-broadcast as success only if this exact transaction ID
            // is already confirmed in our fresh index snapshot.
            if let Some(transaction_id) =
                find_confirmed_transaction_id(&state, &request.transaction_hex).await?
            {
                return Ok(Json(BroadcastTransactionResponse { transaction_id }));
            }
            Err(ApiError::from_broadcast_error(error))
        }
        Err(error) => Err(ApiError::from_broadcast_error(error)),
    }
}

pub(crate) async fn transaction_status(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TransactionStatusRequest>,
) -> Result<Json<TransactionStatusResponse>, ApiError> {
    let id = decode_hash(&request.transaction_id)
        .map_err(|_| ApiError::bad_request("transaction ID must be 64 hexadecimal characters"))?;

    let indexed_status = state
        .read_index(move |reader| reader.read_snapshot(&[])?.query_transaction_status(&id))
        .await?;
    if indexed_status.known {
        return Ok(Json(indexed_status));
    }

    // Fall back to the node for transactions not yet observed by the indexer.
    let node_status = state
        .node
        .fetch_transaction_status(&request.transaction_id)
        .await?;

    let (known, block_height) = match node_status {
        truenorth_rpc::TransactionStatus::Unknown => (false, None),
        truenorth_rpc::TransactionStatus::Pending => (true, None),
        truenorth_rpc::TransactionStatus::Confirmed { block_height } => (true, Some(block_height)),
    };

    Ok(Json(TransactionStatusResponse {
        known,
        block_height,
    }))
}

fn is_missing_inputs_rejection(error: &truenorth_rpc::Error) -> bool {
    matches!(
        error,
        truenorth_rpc::Error::Remote {
            method,
            code: -25,
            message,
        } if method == "sendrawtransaction" && message == "bad-txns-inputs-missingorspent"
    )
}

async fn find_confirmed_transaction_id(
    state: &AppState,
    transaction_hex: &str,
) -> Result<Option<String>, ApiError> {
    let transaction_id = state.node.decode_transaction_id(transaction_hex).await?;
    let id = decode_hash(&transaction_id)?;
    let status = state
        .read_index(move |reader| reader.read_snapshot(&[])?.query_transaction_status(&id))
        .await?;

    if status.block_height.is_some() {
        Ok(Some(transaction_id))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{body::Body, http::Request, http::StatusCode};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use truenorth_rpc::NodeClient;

    use crate::{
        AppState, build_router,
        test_support::{RpcFixture, rejection, request, result, tip},
    };

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
            let router = build_router(state);

            let response = router
                .oneshot(Request::get("/info").body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "application/json");
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            let info: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                info,
                json!({"version": env!("CARGO_PKG_VERSION"), "network": network})
            );
        }
    }

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
            assert!(matches!(
                error,
                truenorth_rpc::Error::Remote { method, code: -26, message }
                if method == "getblockchaininfo" && message == "dust"
            ));
        }

        let malformed_rpc_bodies = [
            json!("not an RPC response"),
            json!({"jsonrpc": "2.0", "id": "northbound", "error": {"code": "-26", "message": "dust"}}),
            json!({"jsonrpc": "2.0", "id": "northbound", "error": {"code": -26}}),
            json!({"jsonrpc": "2.0", "id": "wrong", "error": {"code": -26, "message": "dust"}}),
        ];

        for body in malformed_rpc_bodies {
            let fixture = RpcFixture::new("regtest", move |_| (500, body.clone())).await;
            assert!(matches!(
                fixture.node.fetch_tip().await,
                Err(truenorth_rpc::Error::InvalidData(_))
            ));
        }
    }

    #[tokio::test]
    async fn broadcast_returns_rejection_reasons_and_keeps_upstream_failures_private() {
        let cases = [
            (-22, "TX decode failed", 400),
            (-26, "dust", 400),
            (-26, "txn-mempool-conflict", 409),
            (-25, "bad-txns-inputs-missingorspent", 409),
            (-28, "internal infrastructure detail", 502),
        ];

        for (code, reason, expected_status) in cases {
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

            assert_eq!(status, expected_status);
            if expected_status == 502 {
                assert!(!body.to_string().contains(reason));
            } else {
                assert_eq!(body["message"], reason);
            }
        }

        // Malformed txid from node returns 502.
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

        // Unreachable node returns 502.
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
        let cases = [
            ("main", "main", 200),
            ("regtest", "regtest", 200),
            ("regtest", "main", 502),
        ];

        for (configured, actual, expected_status) in cases {
            let fixture = RpcFixture::new(configured, move |_| (200, tip(actual))).await;
            let (status, body) = request(fixture.router(configured), "/v1/chain/tip", None).await;
            assert_eq!(status, expected_status);
            if status == 200 {
                assert_eq!(body, json!({"height": 12}));
            }
        }
    }

    #[tokio::test]
    async fn status_fallback_distinguishes_unknown_pending_confirmed_and_unavailable() {
        struct StatusCase {
            raw_tx_reply: Value,
            header_reply: Value,
            expected_http: u16,
            expected_known: bool,
            expected_height: Option<u64>,
        }

        let tx_id = "03".repeat(32);
        let block_hash = "01".repeat(32);
        let indexed_coinbase_tx = "02".repeat(32);

        let cases = [
            // Unknown with txindex enabled
            StatusCase {
                raw_tx_reply: rejection(
                    -5,
                    "No such mempool or blockchain transaction. Use gettransaction for wallet transactions.",
                ),
                header_reply: json!(null),
                expected_http: 200,
                expected_known: false,
                expected_height: None,
            },
            // Pending in mempool
            StatusCase {
                raw_tx_reply: result(json!({"txid": tx_id})),
                header_reply: json!(null),
                expected_http: 200,
                expected_known: true,
                expected_height: None,
            },
            // Confirmed in active chain
            StatusCase {
                raw_tx_reply: result(
                    json!({"txid": tx_id, "blockhash": block_hash, "confirmations": 2}),
                ),
                header_reply: result(json!({"hash": block_hash, "confirmations": 2, "height": 12})),
                expected_http: 200,
                expected_known: true,
                expected_height: Some(12),
            },
            // Missing txindex cannot prove absence -> 502
            StatusCase {
                raw_tx_reply: rejection(
                    -5,
                    "No such mempool transaction. Use -txindex or provide a block hash to enable blockchain transaction queries.",
                ),
                header_reply: json!(null),
                expected_http: 502,
                expected_known: false,
                expected_height: None,
            },
            // Mismatched txid -> 502
            StatusCase {
                raw_tx_reply: result(json!({"txid": "04".repeat(32)})),
                header_reply: json!(null),
                expected_http: 502,
                expected_known: false,
                expected_height: None,
            },
            // Negative block height -> 502
            StatusCase {
                raw_tx_reply: result(
                    json!({"txid": tx_id, "blockhash": block_hash, "confirmations": 1}),
                ),
                header_reply: result(json!({"hash": block_hash, "confirmations": 1, "height": -1})),
                expected_http: 502,
                expected_known: false,
                expected_height: None,
            },
            // Zero confirmations with a blockhash -> 502
            StatusCase {
                raw_tx_reply: result(
                    json!({"txid": tx_id, "blockhash": block_hash, "confirmations": 0}),
                ),
                header_reply: json!(null),
                expected_http: 502,
                expected_known: false,
                expected_height: None,
            },
        ];

        for case in cases {
            let expected_tx_id = tx_id.clone();
            let expected_block_hash = block_hash.clone();
            let raw_tx_reply = case.raw_tx_reply;
            let header_reply = case.header_reply;

            let fixture = RpcFixture::new("regtest", move |rpc| {
                let expected_param = if rpc["method"] == "getrawtransaction" {
                    &expected_tx_id
                } else {
                    &expected_block_hash
                };
                assert_eq!(rpc["params"][0], *expected_param);

                match rpc["method"].as_str().unwrap() {
                    "getrawtransaction" => (200, raw_tx_reply.clone()),
                    "getblockheader" => (200, header_reply.clone()),
                    method => panic!("unexpected method: {method}"),
                }
            })
            .await;
            let _database = fixture.create_ready_index();
            let router = fixture.router("regtest");

            // Fallback lookup for a transaction not in the local index.
            let (status, body) = request(
                router.clone(),
                "/v1/transactions/status",
                Some(json!({"transaction_id": tx_id})),
            )
            .await;
            assert_eq!(status, case.expected_http, "{body}");
            if status == 200 {
                assert_eq!(
                    body,
                    json!({"known": case.expected_known, "block_height": case.expected_height})
                );
            }

            // Confirmed transaction already in the index never needs the fallback.
            let (status, body) = request(
                router,
                "/v1/transactions/status",
                Some(json!({"transaction_id": indexed_coinbase_tx})),
            )
            .await;
            assert_eq!(status, 200);
            assert_eq!(body, json!({"known": true, "block_height": 0}));
        }
    }

    #[tokio::test]
    async fn fee_endpoints_preserve_v1_and_return_precise_kvbytes_in_v2() {
        // A controlled RPC rate just above 1 unit/vB exposes premature rounding.
        let fixture = RpcFixture::new("regtest", |rpc| {
            let payload = match rpc["method"].as_str().unwrap() {
                "estimatesmartfee" => json!({"feerate": 0.00001001, "blocks": 6}),
                "getmempoolinfo" => json!({"mempoolminfee": 0.00001000}),
                "getnetworkinfo" => json!({"relayfee": 0.00001000}),
                method => panic!("unexpected RPC method: {method}"),
            };
            (200, result(payload))
        })
        .await;

        let router = fixture.router("regtest");

        let (v1_status, v1_body) = request(router.clone(), "/v1/fees/estimate", None).await;
        assert_eq!(v1_status, 200);
        assert_eq!(
            v1_body,
            json!({"atomic_units_per_vbyte": 2, "target_blocks": 6})
        );

        let (v2_status, v2_body) = request(router, "/v2/fees/estimate", None).await;
        assert_eq!(v2_status, 200);
        assert_eq!(
            v2_body,
            json!({"atomic_units_per_kvbyte": 1001, "target_blocks": 6})
        );
    }
}
