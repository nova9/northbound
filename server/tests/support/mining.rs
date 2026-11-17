use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};

use super::{cli_json, post, run_cli, wait_history};

async fn wait_template(client: &Client, base: &str, parent: &str) -> Value {
    for _ in 0..100 {
        let response = client
            .get(format!("{base}/v1/mining/template"))
            .send()
            .await
            .unwrap();
        if response.status().is_success() {
            let template: Value = response.json().await.unwrap();
            if template["previousblockhash"] == parent {
                return template;
            }
        } else {
            assert_eq!(response.status(), 503);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("mining template never matched parent {parent}");
}

pub async fn exercise_mining_api(client: &Client, base: &str) {
    let parent = run_cli(&["getbestblockhash"]);
    let template = wait_template(client, base, &parent).await;
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
            "{base}/v1/mining/block-hash?height={previous_height}"
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
            .get(format!("{base}/v1/mining/block-hash?height=2147483647"))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );

    let mut waiters = tokio::task::JoinSet::new();
    for _ in 0..2 {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let url = format!("{base}/v1/mining/template");
        let id = template["longpollid"].as_str().unwrap().to_owned();
        waiters.spawn(async move {
            client
                .get(format!("{url}?longpollid={id}"))
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
    let response: Value = post(client, base, "/v1/mining/blocks", json!({"block_hex": hex}))
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
        client,
        base,
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
    let response: Value = post(client, base, "/v1/mining/blocks", json!({"block_hex": hex}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        response,
        json!({"status": "duplicate", "result": "duplicate"})
    );
    let invalid = post(
        client,
        base,
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
    wait_template(client, base, &parent).await;
    let replacement_address = run_cli(&["getnewaddress"]);
    let replacement = cli_json(&["generateblock", &replacement_address, "[]"]);
    assert_ne!(replacement["hash"], mined["hash"]);
    let updated = wait_template(client, base, replacement["hash"].as_str().unwrap()).await;
    assert_eq!(updated["height"], template["height"].as_u64().unwrap() + 1);
}

pub async fn wait_template_transaction(client: &Client, base: &str, txid: &str) {
    for _ in 0..100 {
        let response = client
            .get(format!("{base}/v1/mining/template"))
            .send()
            .await
            .unwrap();
        if response.status().is_success() {
            let template: Value = response.json().await.unwrap();
            if template["transactions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|transaction| transaction["txid"] == txid)
            {
                return;
            }
        } else {
            assert_eq!(response.status(), 503);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("mining template never included pending transaction {txid}");
}

pub async fn wait_unavailable(client: &Client, base: &str) {
    for _ in 0..100 {
        let response = client
            .get(format!("{base}/v1/mining/template"))
            .send()
            .await
            .unwrap();
        if response.status() == 503 {
            return;
        }
        assert_eq!(response.status(), 200);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("mining cache remained available after node shutdown");
}
