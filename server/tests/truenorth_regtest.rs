mod support;

use std::env;
use std::fs::File;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};
use shared::{AddressActivityResponse, TransactionHistory, UtxoResponse};
use support::{
    ServerProcess, broadcast, cli_json, post, run_cli, sign_spend, wait_history, wait_ready,
};

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh server"]
async fn indexed_api_handles_pending_spends_replacements_reorgs_and_restart() {
    assert_eq!(env::var("TRUENORTH_TEST_ISOLATED").as_deref(), Ok("1"));
    assert_eq!(cli_json(&["getblockchaininfo"])["blocks"], 0);
    let directory = tempfile::tempdir().unwrap();
    let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
    let bind = reserved.local_addr().unwrap();
    drop(reserved);
    let url = format!("http://{bind}");
    let cookie =
        PathBuf::from(env::var_os("TRUENORTH_TEST_DATA_DIR").unwrap()).join("regtest/.cookie");
    let node = truenorth_rpc::NodeClient::new(
        &format!(
            "http://127.0.0.1:{}",
            env::var("TRUENORTH_TEST_RPC_PORT").unwrap()
        ),
        cookie.clone(),
        "regtest",
    )
    .unwrap();
    let spawn_server = |mining_enabled: bool| {
        ServerProcess(
            Command::new(env!("CARGO_BIN_EXE_server"))
                .env("TRUENORTH_NETWORK", "regtest")
                .env("TRUENORTH_MINING_ENABLED", mining_enabled.to_string())
                .env(
                    "TRUENORTH_RPC_URL",
                    format!(
                        "http://127.0.0.1:{}",
                        env::var("TRUENORTH_TEST_RPC_PORT").unwrap()
                    ),
                )
                .env("TRUENORTH_RPC_COOKIE_PATH", &cookie)
                .env(
                    "TRUENORTH_INDEX_DATABASE",
                    directory.path().join("chain.sqlite"),
                )
                .env("TRUENORTH_INDEX_POLL_SECONDS", "1")
                .env("TRUENORTH_SERVER_BIND", bind.to_string())
                .stdout(File::create(directory.path().join("server.log")).unwrap())
                .stderr(File::create(directory.path().join("server-error.log")).unwrap())
                .spawn()
                .unwrap(),
        )
    };
    let mut server = spawn_server(false);
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    wait_ready(&client, &url, 0).await;
    assert_eq!(
        client
            .get(format!("{url}/v1/mining/template"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    drop(server);
    server = spawn_server(true);
    wait_ready(&client, &url, 0).await;

    for script in [
        format!("76a914{}88ac", "ab".repeat(20)),
        format!("a914{}87", "cd".repeat(20)),
    ] {
        let mut script_bytes = Vec::new();
        for index in (0..script.len()).step_by(2) {
            script_bytes.push(u8::from_str_radix(&script[index..index + 2], 16).unwrap());
        }
        let address = indexer::address_from_script(&script_bytes, "regtest").unwrap();
        let validated = cli_json(&["validateaddress", &address]);
        assert_eq!(validated["isvalid"], true);
        assert_eq!(validated["scriptPubKey"], script);
    }

    let mining = run_cli(&["getnewaddress"]);
    run_cli(&["generatetoaddress", "101", &mining]);
    wait_ready(&client, &url, 101).await;
    // Node refusals expose their reason instead of looking like a node outage.
    let rejected = post(
        &client,
        &url,
        "/v1/transactions/broadcast",
        json!({"transaction_hex": "00"}),
    )
    .await;
    assert_eq!(rejected.status(), 400);
    assert!(
        rejected.json::<Value>().await.unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("TX decode failed")
    );
    // A single mining address has enough outputs to require multiple pages.
    let mut cursor = None;
    let mut mining_outputs = Vec::new();
    let mut snapshot_revision = None;
    loop {
        let page: UtxoResponse = post(
            &client,
            &url,
            "/v1/utxos",
            json!({"addresses": [mining.clone()], "limit": 17, "cursor": cursor}),
        )
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
        assert!(page.utxos.len() <= 17);
        assert_eq!(page.index.indexed_height, Some(101));
        if let Some(revision) = snapshot_revision {
            assert_eq!(page.index.pending_revision, Some(revision));
        } else {
            snapshot_revision = page.index.pending_revision;
        }
        mining_outputs.extend(page.utxos);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(mining_outputs.len(), 101);
    let complete: UtxoResponse = post(
        &client,
        &url,
        "/v1/utxos",
        json!({"addresses": [mining.clone()]}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert!(complete.next_cursor.is_none());
    assert_eq!(mining_outputs, complete.utxos);
    let first_page: UtxoResponse = post(
        &client,
        &url,
        "/v1/utxos",
        json!({"addresses": [mining.clone()], "limit": 1}),
    )
    .await
    .json()
    .await
    .unwrap();
    let mining_cursor = first_page.next_cursor.unwrap();

    let receiving = run_cli(&["getnewaddress"]);
    let change = run_cli(&["getnewaddress"]);
    let change2 = run_cli(&["getnewaddress"]);
    let recipient = run_cli(&["getnewaddress"]);
    let addresses = vec![receiving.clone(), change.clone(), change2.clone()];
    let payment = run_cli(&[
        "-named",
        "sendtoaddress",
        &format!("address={receiving}"),
        "amount=1",
        "fee_rate=1",
    ]);
    let payment_hex = run_cli(&["getrawtransaction", &payment]);
    assert_eq!(
        post(
            &client,
            &url,
            "/v1/transactions/broadcast",
            json!({"transaction_hex": payment_hex})
        )
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["transaction_id"],
        payment
    );
    assert_eq!(
        node.fetch_transaction_status(&payment).await.unwrap(),
        truenorth_rpc::TransactionStatus::Pending
    );
    let pending = wait_history(&client, &url, &addresses, &payment, 0).await;
    support::mining::wait_template_transaction(&client, &url, &payment).await;
    assert_eq!(pending.transactions[0].net_amount, 100_000_000);
    let status = post(
        &client,
        &url,
        "/v1/transactions/status",
        json!({"transaction_id": payment}),
    )
    .await
    .error_for_status()
    .unwrap()
    .json::<Value>()
    .await
    .unwrap();
    assert_eq!(status, json!({"known": true, "block_height": null}));
    // Spending a reward must not erase its identity in transaction history.
    let decoded_payment = cli_json(&["decoderawtransaction", &payment_hex]);
    let spent_reward = decoded_payment["vin"][0]["txid"].as_str().unwrap();
    let mining_history: TransactionHistory = post(
        &client,
        &url,
        "/v1/transactions",
        json!({"addresses": [mining.clone()], "limit": 200}),
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(
        mining_history
            .transactions
            .iter()
            .any(|tx| tx.transaction_id == spent_reward && tx.coinbase)
    );
    assert!(!pending.transactions[0].coinbase);
    let mining_utxos: UtxoResponse = post(
        &client,
        &url,
        "/v1/utxos",
        json!({"addresses": [mining.clone()]}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert!(
        !mining_utxos
            .utxos
            .iter()
            .any(|output| output.transaction_id == spent_reward)
    );
    // A pending snapshot change invalidates cursors even without a new block.
    assert_eq!(
        post(
            &client,
            &url,
            "/v1/utxos",
            json!({"addresses": [mining.clone()], "cursor": mining_cursor, "limit": 1})
        )
        .await
        .status(),
        409
    );
    let activity: AddressActivityResponse = post(
        &client,
        &url,
        "/v1/addresses/activity",
        json!({"addresses": addresses}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(activity.used_addresses, vec![receiving.clone()]);
    let pending_utxos: UtxoResponse =
        post(&client, &url, "/v1/utxos", json!({"addresses": addresses}))
            .await
            .json()
            .await
            .unwrap();
    assert!(pending_utxos.utxos.is_empty());
    run_cli(&["generatetoaddress", "1", &mining]);
    wait_history(&client, &url, &addresses, &payment, 1).await;
    let confirmed_status = post(
        &client,
        &url,
        "/v1/transactions/status",
        json!({"transaction_id": payment}),
    )
    .await
    .error_for_status()
    .unwrap()
    .json::<Value>()
    .await
    .unwrap();
    assert_eq!(
        confirmed_status,
        json!({"known": true, "block_height": 102})
    );
    let indexed = cli_json(&["getindexinfo"]).get("txindex").is_some();
    if indexed {
        assert_eq!(
            node.fetch_transaction_status(&payment).await.unwrap(),
            truenorth_rpc::TransactionStatus::Confirmed { block_height: 102 }
        );
        assert_eq!(
            node.fetch_transaction_status(&"00".repeat(32))
                .await
                .unwrap(),
            truenorth_rpc::TransactionStatus::Unknown
        );
    } else {
        assert!(
            node.fetch_transaction_status(&payment).await.is_err(),
            "unavailable historical lookup must not mean unknown"
        );
    }
    assert_eq!(
        post(
            &client,
            &url,
            "/v1/transactions/broadcast",
            json!({"transaction_hex": payment_hex})
        )
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["transaction_id"],
        payment
    );
    let utxos: UtxoResponse = post(&client, &url, "/v1/utxos", json!({"addresses": addresses}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(utxos.utxos.len(), 1);
    let original = &utxos.utxos[0];
    assert_eq!(original.amount, 100_000_000);
    let oracle = cli_json(&[
        "gettxout",
        &payment,
        &original.output_index.to_string(),
        "false",
    ]);
    assert_eq!(oracle["scriptPubKey"]["hex"], original.script_pubkey);

    let signed_parent = sign_spend(
        &payment,
        original.output_index,
        json!([{recipient.clone(): 0.5}, {change.clone(): 0.49999800}]),
    );
    let parent = broadcast(&client, &url, &signed_parent).await;
    assert_eq!(
        post(
            &client,
            &url,
            "/v1/transactions/broadcast",
            json!({"transaction_hex": signed_parent})
        )
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["transaction_id"],
        parent
    );
    let conflict_hex = sign_spend(
        &payment,
        original.output_index,
        json!([{recipient.clone(): 0.5}, {change.clone(): 0.49999900}]),
    );
    let conflict = post(
        &client,
        &url,
        "/v1/transactions/broadcast",
        json!({"transaction_hex": conflict_hex}),
    )
    .await;
    assert_eq!(conflict.status(), 400);
    assert!(
        conflict.json::<Value>().await.unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("fee")
    );
    let history = wait_history(&client, &url, &addresses, &parent, 0).await;
    let parent_history = history
        .transactions
        .iter()
        .find(|tx| tx.transaction_id == parent)
        .unwrap();
    assert_eq!(parent_history.net_amount, -50_000_200);
    assert_eq!(parent_history.network_fee, Some(200));
    let mut combined_net_amount = 0_i64;
    for address in [&receiving, &change] {
        let batch: TransactionHistory = post(
            &client,
            &url,
            "/v1/transactions",
            json!({"addresses": [address], "limit": 100}),
        )
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(batch.index.pending_revision, history.index.pending_revision);
        let transaction = batch
            .transactions
            .iter()
            .find(|tx| tx.transaction_id == parent)
            .unwrap();
        assert_eq!(transaction.network_fee, Some(200));
        combined_net_amount += transaction.net_amount;
    }
    assert_eq!(combined_net_amount, parent_history.net_amount);
    let utxos: UtxoResponse = post(&client, &url, "/v1/utxos", json!({"addresses": addresses}))
        .await
        .json()
        .await
        .unwrap();
    assert!(
        utxos.utxos.is_empty(),
        "pending spend must prevent reselection"
    );
    let signed_child = sign_spend(&parent, 1, json!([{change2.clone(): 0.49999600}]));
    let child = broadcast(&client, &url, &signed_child).await;
    let history = wait_history(&client, &url, &addresses, &child, 0).await;
    assert!(
        history
            .transactions
            .iter()
            .any(|tx| tx.transaction_id == parent)
    );

    // RBF removes both the old parent and its dependent child from one snapshot.
    let replacement_hex = sign_spend(
        &payment,
        original.output_index,
        json!([{recipient.clone(): 0.5}, {change.clone(): 0.49999000}]),
    );
    let replacement = broadcast(&client, &url, &replacement_hex).await;
    let history = wait_history(&client, &url, &addresses, &replacement, 0).await;
    assert!(
        !history
            .transactions
            .iter()
            .any(|tx| tx.transaction_id == parent || tx.transaction_id == child)
    );
    let mined = cli_json(&["generatetoaddress", "1", &mining]);
    let confirmed = wait_history(&client, &url, &addresses, &replacement, 1).await;
    assert_eq!(
        confirmed
            .transactions
            .iter()
            .filter(|tx| tx.transaction_id == replacement)
            .count(),
        1
    );

    // The replacement spends everything from receiving, with no change to it.
    let receiving_utxos: UtxoResponse = post(
        &client,
        &url,
        "/v1/utxos",
        json!({"addresses": [receiving.clone()]}),
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(receiving_utxos.utxos.is_empty());
    let no_change_status = post(
        &client,
        &url,
        "/v1/transactions/status",
        json!({"transaction_id": replacement}),
    )
    .await
    .error_for_status()
    .unwrap()
    .json::<Value>()
    .await
    .unwrap();
    assert_eq!(
        no_change_status,
        json!({"known": true, "block_height": 103})
    );
    let conflict = post(
        &client,
        &url,
        "/v1/transactions/broadcast",
        json!({"transaction_hex": signed_parent}),
    )
    .await;
    assert_eq!(conflict.status(), 409);
    assert!(
        conflict.json::<Value>().await.unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("missingorspent")
    );
    // All outputs of the original payment are now spent; an exact repeat still succeeds.
    assert_eq!(
        post(
            &client,
            &url,
            "/v1/transactions/broadcast",
            json!({"transaction_hex": payment_hex})
        )
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["transaction_id"],
        payment
    );
    run_cli(&["invalidateblock", mined[0].as_str().unwrap()]);
    run_cli(&["generateblock", &mining, "[]"]);
    wait_history(&client, &url, &addresses, &replacement, 0).await;
    run_cli(&["generatetoaddress", "1", &mining]);
    wait_history(&client, &url, &addresses, &replacement, 1).await;

    let first: TransactionHistory = post(
        &client,
        &url,
        "/v1/transactions",
        json!({"addresses": addresses, "limit": 1}),
    )
    .await
    .json()
    .await
    .unwrap();
    let cursor = first.next_cursor.unwrap();
    let second: TransactionHistory = post(
        &client,
        &url,
        "/v1/transactions",
        json!({"addresses": addresses, "limit": 1, "cursor": cursor.clone()}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_ne!(
        first.transactions[0].transaction_id,
        second.transactions[0].transaction_id
    );
    run_cli(&["generatetoaddress", "1", &mining]);
    wait_ready(&client, &url, 105).await;
    let response = post(
        &client,
        &url,
        "/v1/transactions",
        json!({"addresses": addresses, "cursor": cursor}),
    )
    .await;
    assert_eq!(response.status(), 409);

    for body in [
        json!({"addresses": ["not-an-address"]}),
        json!({"addresses": vec![receiving.clone(); 101]}),
    ] {
        for endpoint in ["/v1/utxos", "/v1/addresses/activity", "/v1/transactions"] {
            assert_eq!(
                post(&client, &url, endpoint, body.clone()).await.status(),
                400
            );
        }
    }
    assert_eq!(
        post(
            &client,
            &url,
            "/v1/transactions/broadcast",
            json!({"transaction_hex": "zz"})
        )
        .await
        .status(),
        400
    );
    for body in [
        json!({"addresses": addresses, "limit": 0}),
        json!({"addresses": addresses, "limit": 10_001}),
        json!({"addresses": addresses, "cursor": "invalid"}),
    ] {
        assert_eq!(post(&client, &url, "/v1/utxos", body).await.status(), 400);
    }
    let malformed = client
        .post(format!("{url}/v1/utxos"))
        .header("content-type", "application/json")
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), 400);
    assert!(malformed.json::<Value>().await.unwrap()["message"].is_string());
    let fee: Value = client
        .get(format!("{url}/v1/fees/estimate"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(fee["atomic_units_per_vbyte"].as_u64().unwrap() > 0);
    let precise_fee: Value = client
        .get(format!("{url}/v2/fees/estimate"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let estimate = cli_json(&["estimatesmartfee", "6"]);
    let mempool = cli_json(&["getmempoolinfo"]);
    let network = cli_json(&["getnetworkinfo"]);
    let mut expected_rate = 0;
    for value in [
        &estimate["feerate"],
        &mempool["mempoolminfee"],
        &network["relayfee"],
    ] {
        if value.is_null() {
            continue;
        }
        let rate = truenorth_rpc::atomic_units(value.as_number().unwrap()).unwrap() as u64;
        expected_rate = expected_rate.max(rate);
    }
    assert_eq!(
        precise_fee,
        json!({"atomic_units_per_kvbyte": expected_rate, "target_blocks": 6})
    );
    assert_eq!(
        fee,
        json!({"atomic_units_per_vbyte": expected_rate.div_ceil(1000), "target_blocks": 6})
    );
    let tip: Value = client
        .get(format!("{url}/v1/chain/tip"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tip["height"], 105);
    assert_eq!(tip, json!({"height": 105}));
    let info: Value = client
        .get(format!("{url}/info"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        info,
        json!({"version": env!("CARGO_PKG_VERSION"), "network": "regtest"})
    );
    let known: Value = post(
        &client,
        &url,
        "/v1/transactions/status",
        json!({"transaction_id": payment}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(
        known["known"], true,
        "confirmed lookup must work without txindex"
    );

    assert_eq!(known["block_height"], 102);

    drop(server);
    server = spawn_server(true);
    wait_ready(&client, &url, 105).await;
    let resumed = wait_history(&client, &url, &addresses, &replacement, 2).await;
    assert_eq!(
        resumed
            .transactions
            .iter()
            .filter(|tx| tx.transaction_id == replacement)
            .count(),
        1
    );

    support::mining::exercise_mining_api(&client, &url).await;
    run_cli(&["stop"]);
    for _ in 0..200 {
        if client
            .get(format!("{url}/ready"))
            .send()
            .await
            .unwrap()
            .status()
            == 503
        {
            let response = post(&client, &url, "/v1/utxos", json!({"addresses": addresses})).await;
            assert_eq!(response.status(), 503);
            assert!(response.json::<Value>().await.unwrap()["message"].is_string());
            support::mining::wait_unavailable(&client, &url).await;
            drop(server);
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("node failure did not invalidate readiness");
}
