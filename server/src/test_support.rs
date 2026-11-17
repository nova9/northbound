use std::num::NonZeroU32;
use std::sync::Arc;

use axum::{Json, Router, body::Body, http::Request, routing::post};
use serde_json::{Value, json};
use tower::ServiceExt;
use truenorth_rpc::NodeClient;

use crate::AppState;

pub struct RpcFixture {
    pub node: NodeClient,
    pub directory: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for RpcFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl RpcFixture {
    pub async fn new(
        network: &str,
        reply: impl Fn(Value) -> (u16, Value) + Send + Sync + 'static,
    ) -> Self {
        Self::new_raw(network, move |request| {
            let (status, body) = reply(request);
            (status, body.to_string())
        })
        .await
    }

    pub async fn new_raw(
        network: &str,
        reply: impl Fn(Value) -> (u16, String) + Send + Sync + 'static,
    ) -> Self {
        let reply = Arc::new(reply);
        let rpc = Router::new().route(
            "/",
            post(move |Json(request): Json<Value>| {
                let (status, body) = reply(request);
                async move { (axum::http::StatusCode::from_u16(status).unwrap(), body) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        let cookie = directory.path().join(".cookie");
        std::fs::write(&cookie, "test:password").unwrap();
        let node = NodeClient::new(&url, cookie, network).unwrap();
        Self {
            node,
            directory,
            task,
        }
    }

    pub fn task_abort(&self) {
        self.task.abort();
    }

    pub fn router(&self, network: &str) -> Router {
        crate::build_router(
            Arc::new(AppState::new(
                self.node.clone(),
                self.directory.path().join("chain.sqlite"),
                network.into(),
            )),
            NonZeroU32::new(100).unwrap(),
        )
    }

    pub fn create_ready_index(&self) -> indexer::IndexDatabase {
        let mut database = indexer::IndexDatabase::open(
            &self.directory.path().join("chain.sqlite"),
            "regtest",
            [1; 32],
        )
        .unwrap();
        let block = truenorth_rpc::decode_block(
            &json!({
                "hash": "01".repeat(32), "height": 0, "time": 1,
                "tx": [{"txid": "02".repeat(32), "vin": [{"coinbase": "00"}],
                    "vout": [{"n": 0, "value": 512, "scriptPubKey": {"hex": "51"}}]}]
            })
            .to_string(),
        )
        .unwrap();
        database.apply_block(&block).unwrap();
        database
            .publish_mempool_snapshot(&database.tip().unwrap().unwrap(), 1, &[])
            .unwrap();
        database
    }
}

pub fn result(value: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": "northbound", "result": value, "error": null})
}

pub fn rejection(code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": "northbound", "error": {"code": code, "message": message}})
}

pub fn tip(network: &str) -> Value {
    result(
        json!({"chain": network, "blocks": 12, "bestblockhash": "01".repeat(32), "pruned": false}),
    )
}

pub async fn request(router: Router, path: &str, body: Option<Value>) -> (u16, Value) {
    let request = match body {
        Some(body) => Request::post(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => Request::get(path).body(Body::empty()).unwrap(),
    };
    let response = router.oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
