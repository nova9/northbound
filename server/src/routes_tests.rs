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
            let fixture = RpcFixture::new_raw("regtest", move |_| (http_status, body.into())).await;

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

#[tokio::test]
async fn invalid_request_inputs_return_bad_request() {
    let fixture = RpcFixture::new("regtest", |_| (200, tip("regtest"))).await;
    let _database = fixture.create_ready_index();
    let router = fixture.router("regtest");
    let mut p2pkh_script = vec![0x76, 0xa9, 0x14];
    p2pkh_script.extend([0xab; 20]);
    p2pkh_script.extend([0x88, 0xac]);
    let valid_address = indexer::address_from_script(&p2pkh_script, "regtest").unwrap();

    for body in [
        json!({"addresses": ["not-an-address"]}),
        json!({"addresses": vec![valid_address.clone(); 101]}),
    ] {
        for endpoint in ["/v1/utxos", "/v1/addresses/activity", "/v1/transactions"] {
            let (status, _) = request(router.clone(), endpoint, Some(body.clone())).await;
            assert_eq!(status, 400, "{endpoint} should reject {body}");
        }
    }

    let (status, _) = request(
        router.clone(),
        "/v1/transactions/broadcast",
        Some(json!({"transaction_hex": "zz"})),
    )
    .await;
    assert_eq!(status, 400);

    for body in [
        json!({"addresses": [valid_address.clone()], "limit": 0}),
        json!({"addresses": [valid_address.clone()], "limit": 10_001}),
        json!({"addresses": [valid_address], "cursor": "invalid"}),
    ] {
        let (status, _) = request(router.clone(), "/v1/utxos", Some(body.clone())).await;
        assert_eq!(status, 400, "/v1/utxos should reject {body}");
    }

    let malformed = tower::ServiceExt::oneshot(
        router,
        axum::http::Request::post("/v1/utxos")
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{"))
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(malformed.status(), 400);
    let bytes = axum::body::to_bytes(malformed.into_body(), 4096)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["message"].is_string());
}
