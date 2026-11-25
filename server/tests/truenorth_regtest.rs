mod support;

use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};
use shared::{AddressActivityResponse, TransactionHistory, UtxoResponse};
use support::mining::{wait_template, wait_template_transaction, wait_unavailable};
use support::{
    broadcast, cli_json, current_height, ensure_spendable_funds, http_client, node_client, post,
    reserve_local_addr, run_cli, sign_spend, spawn_server, spawn_server_with_rpc, start_rpc_proxy,
    wait_history, wait_ready,
};

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh server"]
async fn mining_disabled_hides_routes_and_legacy_scripts_validate() {
    let directory = tempfile::tempdir().unwrap();
    let bind = reserve_local_addr();
    let url = format!("http://{bind}");
    let client = http_client();

    let _server = spawn_server(directory.path(), bind, false);
    wait_ready(&client, &url, current_height()).await;
    assert_eq!(
        client
            .get(format!("{url}/v1/mining/template"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );

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
}

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh server"]
async fn utxo_pagination_and_cursor_invalidation() {
    let directory = tempfile::tempdir().unwrap();
    let bind = reserve_local_addr();
    let url = format!("http://{bind}");
    let client = http_client();
    let _server = spawn_server(directory.path(), bind, true);

    // Mine 25 blocks to a fresh address so pagination spans multiple pages.
    let mining = run_cli(&["getnewaddress"]);
    run_cli(&["generatetoaddress", "25", &mining]);
    let height = current_height();
    wait_ready(&client, &url, height).await;

    let mut cursor = None;
    let mut mining_outputs = Vec::new();
    let mut snapshot_revision = None;
    loop {
        let page: UtxoResponse = post(
            &client,
            &url,
            "/v1/utxos",
            json!({"addresses": [mining.clone()], "limit": 7, "cursor": cursor}),
        )
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
        assert!(page.utxos.len() <= 7);
        assert_eq!(page.index.indexed_height, Some(height));
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
    assert_eq!(mining_outputs.len(), 25);

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
    let stale_cursor = first_page.next_cursor.unwrap();

    // Ensure spendable funds exist, then create a mempool change without a new block.
    ensure_spendable_funds();
    let receiving = run_cli(&["getnewaddress"]);
    let payment = run_cli(&[
        "-named",
        "sendtoaddress",
        &format!("address={receiving}"),
        "amount=1",
        "fee_rate=1",
    ]);
    wait_history(&client, &url, &[receiving], &payment, 0).await;

    // A pending snapshot change invalidates cursors even without a new block.
    assert_eq!(
        post(
            &client,
            &url,
            "/v1/utxos",
            json!({"addresses": [mining.clone()], "cursor": stale_cursor, "limit": 1})
        )
        .await
        .status(),
        409
    );

    // Clean up the pending transaction so subsequent tests start with a clean mempool.
    run_cli(&["generatetoaddress", "1", &mining]);
}

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh server"]
async fn pending_spends_rbf_reorg_and_status_lookup() {
    let directory = tempfile::tempdir().unwrap();
    let bind = reserve_local_addr();
    let url = format!("http://{bind}");
    let client = http_client();
    let node = node_client();

    let mining = ensure_spendable_funds();
    let mut server = spawn_server(directory.path(), bind, true);
    wait_ready(&client, &url, current_height()).await;

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
    wait_template_transaction(&client, &url, &payment).await;
    assert_eq!(pending.transactions[0].net_amount, 100_000_000);
    assert!(!pending.transactions[0].coinbase);

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
    let spent_reward_tx = cli_json(&["gettransaction", spent_reward]);
    let spent_reward_address = spent_reward_tx["details"][0]["address"]
        .as_str()
        .unwrap()
        .to_owned();
    let reward_history: TransactionHistory = post(
        &client,
        &url,
        "/v1/transactions",
        json!({"addresses": [spent_reward_address.clone()], "limit": 200}),
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(
        reward_history
            .transactions
            .iter()
            .any(|tx| tx.transaction_id == spent_reward && tx.coinbase)
    );
    let reward_utxos: UtxoResponse = post(
        &client,
        &url,
        "/v1/utxos",
        json!({"addresses": [spent_reward_address]}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert!(
        !reward_utxos
            .utxos
            .iter()
            .any(|output| output.transaction_id == spent_reward)
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
    let payment_height = current_height();
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
        json!({"known": true, "block_height": payment_height})
    );

    let indexed = cli_json(&["getindexinfo"]).get("txindex").is_some();
    if indexed {
        assert_eq!(
            node.fetch_transaction_status(&payment).await.unwrap(),
            truenorth_rpc::TransactionStatus::Confirmed {
                block_height: payment_height
            }
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
    let replacement_height = current_height();
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
        json!({"known": true, "block_height": replacement_height})
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

    // Reorg moves replacement back to mempool, then re-confirms on the new branch.
    run_cli(&["invalidateblock", mined[0].as_str().unwrap()]);
    run_cli(&["generateblock", &mining, "[]"]);
    wait_history(&client, &url, &addresses, &replacement, 0).await;
    run_cli(&["generatetoaddress", "1", &mining]);
    wait_history(&client, &url, &addresses, &replacement, 1).await;

    // History pagination works within one snapshot and returns 409 after a new block.
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
    let tip_height = current_height();
    wait_ready(&client, &url, tip_height).await;
    let response = post(
        &client,
        &url,
        "/v1/transactions",
        json!({"addresses": addresses, "cursor": cursor}),
    )
    .await;
    assert_eq!(response.status(), 409);

    // Confirmed status lookup works without txindex and survives a server restart.
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
    assert_eq!(known["block_height"], payment_height);

    drop(server);
    server = spawn_server(directory.path(), bind, true);
    wait_ready(&client, &url, tip_height).await;
    let resumed = wait_history(&client, &url, &addresses, &replacement, 2).await;
    assert_eq!(
        resumed
            .transactions
            .iter()
            .filter(|tx| tx.transaction_id == replacement)
            .count(),
        1
    );
    drop(server);
}

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh server"]
async fn fee_estimation_info_and_chain_tip_match_core() {
    let directory = tempfile::tempdir().unwrap();
    let bind = reserve_local_addr();
    let url = format!("http://{bind}");
    let client = http_client();
    let _server = spawn_server(directory.path(), bind, true);

    let tip_height = current_height();
    wait_ready(&client, &url, tip_height).await;

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
    assert_eq!(tip, json!({"height": tip_height}));

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
}

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh server"]
async fn mining_template_submission_longpoll_and_reorg() {
    let directory = tempfile::tempdir().unwrap();
    let bind = reserve_local_addr();
    let url = format!("http://{bind}");
    let client = http_client();
    let _server = spawn_server(directory.path(), bind, true);
    wait_ready(&client, &url, current_height()).await;

    let parent = run_cli(&["getbestblockhash"]);
    let template = wait_template(&client, &url, &parent).await;
    let oracle = cli_json(&["getblocktemplate", "{\"rules\":[\"segwit\"]}"]);
    for field in [
        "height",
        "previousblockhash",
        "bits",
        "target",
        "coinbasevalue",
        "rules",
        "transactions",
        "default_witness_commitment",
    ] {
        assert_eq!(template[field], oracle[field], "template field {field}");
    }

    let previous_height = template["height"].as_u64().unwrap() - 1;
    let hash: Value = client
        .get(format!(
            "{url}/v1/mining/block-hash?height={previous_height}"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hash, json!({"height": previous_height, "hash": parent}));
    assert_eq!(
        client
            .get(format!("{url}/v1/mining/block-hash?height=2147483647"))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );

    let mut waiters = tokio::task::JoinSet::new();
    for _ in 0..2 {
        let waiter_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let template_url = format!("{url}/v1/mining/template");
        let id = template["longpollid"].as_str().unwrap().to_owned();
        waiters.spawn(async move {
            waiter_client
                .get(format!("{template_url}?longpollid={id}"))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        });
    }

    // Core mines a real RandomX block locally but leaves submission to our API.
    let address = run_cli(&["getnewaddress"]);
    let mined = cli_json(&["generateblock", &address, "[]", "false"]);
    assert_eq!(run_cli(&["getbestblockhash"]), parent);
    let hex = mined["hex"].as_str().unwrap();
    let response: Value = post(
        &client,
        &url,
        "/v1/mining/blocks",
        json!({"block_hex": hex}),
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(response, json!({"status": "accepted", "result": null}));
    let hash = mined["hash"].as_str().unwrap();
    assert_eq!(run_cli(&["getbestblockhash"]), hash);

    let block = cli_json(&["getblock", hash, "2"]);
    let coinbase = &block["tx"][0];
    assert_eq!(
        coinbase["vout"][0]["scriptPubKey"]["hex"],
        cli_json(&["validateaddress", &address])["scriptPubKey"]
    );
    let reward =
        truenorth_rpc::atomic_units(coinbase["vout"][0]["value"].as_number().unwrap()).unwrap();
    // This generated block has no transactions, so its reward must not exceed
    // the template's subsidy-plus-fees allowance.
    assert!(reward > 0 && reward as u64 <= template["coinbasevalue"].as_u64().unwrap());
    assert_eq!(
        cli_json(&["gettxout", coinbase["txid"].as_str().unwrap(), "0"])["coinbase"],
        true
    );

    let history = wait_history(
        &client,
        &url,
        &[address.clone()],
        coinbase["txid"].as_str().unwrap(),
        1,
    )
    .await;
    let reward = history
        .transactions
        .iter()
        .find(|transaction| transaction.transaction_id == coinbase["txid"].as_str().unwrap())
        .unwrap();
    assert!(reward.coinbase);
    while let Some(updated) = waiters.join_next().await {
        assert_eq!(updated.unwrap()["previousblockhash"], hash);
    }

    let response: Value = post(
        &client,
        &url,
        "/v1/mining/blocks",
        json!({"block_hex": hex}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(
        response,
        json!({"status": "duplicate", "result": "duplicate"})
    );

    let invalid = post(
        &client,
        &url,
        "/v1/mining/blocks",
        json!({"block_hex": "00".repeat(81)}),
    )
    .await
    .error_for_status()
    .unwrap()
    .json::<Value>()
    .await
    .unwrap();
    assert_eq!(invalid["status"], "rejected");
    assert!(invalid["result"].is_string());

    // Keep the original height while replacing its block, proving cache updates
    // use the parent hash rather than height alone.
    run_cli(&["invalidateblock", hash]);
    wait_template(&client, &url, &parent).await;
    let replacement_address = run_cli(&["getnewaddress"]);
    let replacement = cli_json(&["generateblock", &replacement_address, "[]"]);
    assert_ne!(replacement["hash"], mined["hash"]);
    let updated = wait_template(&client, &url, replacement["hash"].as_str().unwrap()).await;
    assert_eq!(updated["height"], template["height"].as_u64().unwrap() + 1);
}

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh server"]
async fn node_outage_invalidates_readiness_and_mining_cache() {
    let directory = tempfile::tempdir().unwrap();
    let bind = reserve_local_addr();
    let url = format!("http://{bind}");
    let client = http_client();
    let proxy = start_rpc_proxy().await;
    let _server = spawn_server_with_rpc(directory.path(), bind, &proxy.url, true);

    let height = current_height();
    wait_ready(&client, &url, height).await;
    let parent = run_cli(&["getbestblockhash"]);
    wait_template(&client, &url, &parent).await;

    // Sever the server's RPC connection without stopping the shared regtest node.
    proxy.stop();
    let address = run_cli(&["getnewaddress"]);

    for _ in 0..200 {
        if client
            .get(format!("{url}/ready"))
            .send()
            .await
            .unwrap()
            .status()
            == 503
        {
            let response = post(&client, &url, "/v1/utxos", json!({"addresses": [address]})).await;
            assert_eq!(response.status(), 503);
            assert!(response.json::<Value>().await.unwrap()["message"].is_string());
            wait_unavailable(&client, &url).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("node failure did not invalidate readiness");
}
