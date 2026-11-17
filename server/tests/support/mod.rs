use std::env;
use std::process::{Child, Command};
use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};
use shared::{IndexStatus, TransactionHistory};

pub mod mining;

pub struct ServerProcess(pub Child);

impl Drop for ServerProcess {
    fn drop(&mut self) {
        if self.0.try_wait().is_ok_and(|status| status.is_none()) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

pub fn run_cli(arguments: &[&str]) -> String {
    assert_eq!(env::var("TRUENORTH_TEST_ISOLATED").as_deref(), Ok("1"));
    let output = Command::new(env::var_os("TRUENORTH_TEST_CLI").unwrap())
        .arg("-regtest")
        .arg(format!(
            "-datadir={}",
            env::var("TRUENORTH_TEST_DATA_DIR").unwrap()
        ))
        .arg(format!(
            "-rpcport={}",
            env::var("TRUENORTH_TEST_RPC_PORT").unwrap()
        ))
        .arg("-rpcwallet=funding")
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}: {}",
        arguments[0],
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

pub fn cli_json(arguments: &[&str]) -> Value {
    serde_json::from_str(&run_cli(arguments)).unwrap()
}

pub async fn post(client: &Client, base: &str, path: &str, body: Value) -> reqwest::Response {
    client
        .post(format!("{base}{path}"))
        .json(&body)
        .send()
        .await
        .unwrap()
}

pub async fn wait_ready(client: &Client, base: &str, height: u64) {
    for _ in 0..400 {
        if let Ok(response) = client.get(format!("{base}/ready")).send().await
            && response.status().is_success()
        {
            let status: IndexStatus = response.json().await.unwrap();
            if status.ready && status.indexed_height == Some(height) {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("server never became ready at height {height}");
}

pub async fn wait_history(
    client: &Client,
    base: &str,
    addresses: &[String],
    txid: &str,
    confirmations: u64,
) -> TransactionHistory {
    for _ in 0..400 {
        let response = post(
            client,
            base,
            "/v1/transactions",
            json!({"addresses": addresses}),
        )
        .await;
        if response.status().is_success() {
            let history: TransactionHistory = response.json().await.unwrap();
            if history
                .transactions
                .iter()
                .any(|tx| tx.transaction_id == txid && tx.confirmations == confirmations)
            {
                return history;
            }
        } else {
            assert_eq!(response.status(), 503, "{}", response.text().await.unwrap());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("transaction {txid} never reached {confirmations} confirmations in API history");
}

pub fn sign_spend(txid: &str, vout: u32, outputs: Value) -> String {
    let inputs = json!([{"txid": txid, "vout": vout}]);
    let raw = run_cli(&[
        "createrawtransaction",
        &inputs.to_string(),
        &outputs.to_string(),
        "0",
        "true",
    ]);
    let signed = cli_json(&["signrawtransactionwithwallet", &raw]);
    assert_eq!(signed["complete"], true);
    signed["hex"].as_str().unwrap().into()
}

pub async fn broadcast(client: &Client, base: &str, hex: &str) -> String {
    let response = post(
        client,
        base,
        "/v1/transactions/validate",
        json!({"transaction_hex": hex}),
    )
    .await;
    assert!(response.status().is_success());
    assert_eq!(response.json::<Value>().await.unwrap()["allowed"], true);
    let response = post(
        client,
        base,
        "/v1/transactions/broadcast",
        json!({"transaction_hex": hex}),
    )
    .await;
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    response.json::<Value>().await.unwrap()["transaction_id"]
        .as_str()
        .unwrap()
        .into()
}
