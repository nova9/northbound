use std::env;
use std::fs::File;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};
use shared::{IndexStatus, TransactionHistory};
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener as AsyncTcpListener, TcpStream};

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
    assert_eq!(
        env::var("TRUENORTH_TEST_ISOLATED").as_deref(),
        Ok("1"),
        "run scripts/test-truenorth-integration.sh server; tests require an isolated node"
    );
    let cli_path =
        env::var_os("TRUENORTH_TEST_CLI").expect("TRUENORTH_TEST_CLI must point to truenorth-cli");
    let data_directory = env::var("TRUENORTH_TEST_DATA_DIR")
        .expect("TRUENORTH_TEST_DATA_DIR must point to the test node data");
    let rpc_port =
        env::var("TRUENORTH_TEST_RPC_PORT").expect("TRUENORTH_TEST_RPC_PORT must be set");
    let funding_wallet = env::var("TRUENORTH_TEST_WALLET")
        .expect("TRUENORTH_TEST_WALLET must name the funding wallet");

    let output = Command::new(cli_path)
        .arg("-regtest")
        .arg(format!("-datadir={data_directory}"))
        .arg(format!("-rpcport={rpc_port}"))
        .arg(format!("-rpcwallet={funding_wallet}"))
        .args(arguments)
        .output()
        .expect("truenorth-cli should start");
    assert!(
        output.status.success(),
        "{}: {}",
        arguments[0],
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("truenorth-cli output should be UTF-8")
        .trim()
        .to_owned()
}

pub fn cli_json(arguments: &[&str]) -> Value {
    serde_json::from_str(&run_cli(arguments)).expect("truenorth-cli output should be valid JSON")
}

pub fn current_height() -> u64 {
    cli_json(&["getblockchaininfo"])["blocks"]
        .as_u64()
        .expect("blockchain info should include block height")
}

pub fn cookie_path() -> PathBuf {
    PathBuf::from(env::var_os("TRUENORTH_TEST_DATA_DIR").unwrap()).join("regtest/.cookie")
}

pub fn rpc_url() -> String {
    format!(
        "http://127.0.0.1:{}",
        env::var("TRUENORTH_TEST_RPC_PORT").unwrap()
    )
}

pub fn node_client() -> truenorth_rpc::NodeClient {
    truenorth_rpc::NodeClient::new(&rpc_url(), cookie_path(), "regtest").unwrap()
}

pub fn reserve_local_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

pub fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

pub fn spawn_server(directory: &Path, bind: SocketAddr, mining_enabled: bool) -> ServerProcess {
    spawn_server_with_rpc(directory, bind, &rpc_url(), mining_enabled)
}

pub fn spawn_server_with_rpc(
    directory: &Path,
    bind: SocketAddr,
    rpc_url: &str,
    mining_enabled: bool,
) -> ServerProcess {
    ServerProcess(
        Command::new(env!("CARGO_BIN_EXE_server"))
            .env("TRUENORTH_NETWORK", "regtest")
            .env("TRUENORTH_MINING_ENABLED", mining_enabled.to_string())
            .env("TRUENORTH_RPC_URL", rpc_url)
            .env("TRUENORTH_RPC_COOKIE_PATH", cookie_path())
            .env("TRUENORTH_INDEX_DATABASE", directory.join("chain.sqlite"))
            .env("TRUENORTH_INDEX_POLL_SECONDS", "1")
            .env("TRUENORTH_SERVER_BIND", bind.to_string())
            .stdout(File::create(directory.join("server.log")).unwrap())
            .stderr(File::create(directory.join("server-error.log")).unwrap())
            .spawn()
            .unwrap(),
    )
}

/// Ensures the funding wallet has mature spendable coinbase outputs.
pub fn ensure_spendable_funds() -> String {
    let mining = run_cli(&["getnewaddress"]);
    if current_height() < 101 {
        let needed = 101 - current_height();
        run_cli(&["generatetoaddress", &needed.to_string(), &mining]);
    }
    mining
}

pub struct RpcProxy {
    pub url: String,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl RpcProxy {
    pub fn stop(&self) {
        let _ = self.shutdown.send(true);
    }
}

impl Drop for RpcProxy {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Proxies TCP connections to the isolated regtest node so a test can simulate
/// node outage by dropping the proxy (closing both the listener and active
/// HTTP keep-alive connections) without stopping the shared daemon.
pub async fn start_rpc_proxy() -> RpcProxy {
    let upstream = format!("127.0.0.1:{}", env::var("TRUENORTH_TEST_RPC_PORT").unwrap());
    let listener = AsyncTcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (shutdown, mut shutdown_rx) = tokio::sync::watch::channel(false);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => break,
                accepted = listener.accept() => {
                    let Ok((mut inbound, _)) = accepted else {
                        break;
                    };
                    let upstream = upstream.clone();
                    let mut conn_shutdown = shutdown_rx.clone();
                    tokio::spawn(async move {
                        if let Ok(mut outbound) = TcpStream::connect(&upstream).await {
                            tokio::select! {
                                _ = conn_shutdown.changed() => {}
                                _ = copy_bidirectional(&mut inbound, &mut outbound) => {}
                            }
                        }
                    });
                }
            }
        }
    });

    RpcProxy { url, shutdown }
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
