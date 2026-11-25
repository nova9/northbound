use std::time::Duration;

use reqwest::Client;
use serde_json::Value;

pub async fn wait_template(client: &Client, base: &str, parent: &str) -> Value {
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
