use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Block, Error, Hash, Result, Transaction, decode_block, decode_hash, decode_transaction,
};

const MAX_RPC_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct NodeClient {
    url: reqwest::Url,
    cookie_path: PathBuf,
    network: String,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
pub struct NodeTip {
    pub chain: String,
    pub blocks: i64,
    pub bestblockhash: String,
    pub pruned: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct MempoolIds {
    pub txids: Vec<String>,
    pub mempool_sequence: u64,
}

impl NodeClient {
    pub fn new(url: &str, cookie_path: PathBuf, network: &str) -> Result<Self> {
        if network != "main" && network != "regtest" {
            return Err(Error::InvalidData("network must be main or regtest".into()));
        }
        let url = reqwest::Url::parse(url)
            .map_err(|_| Error::InvalidData("RPC URL is invalid".into()))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::InvalidData(
                "RPC URL must use HTTP(S) without embedded credentials, query, or fragment".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Transport("could not create HTTP client".into()))?;
        Ok(Self {
            url,
            cookie_path,
            network: network.to_owned(),
            http,
        })
    }

    pub async fn fetch_tip(&self) -> Result<NodeTip> {
        let value = self.call_rpc("getblockchaininfo", json!([])).await?;
        let tip: NodeTip = serde_json::from_value(value)
            .map_err(|_| Error::InvalidData("invalid blockchain info response".into()))?;
        if tip.chain != self.network {
            return Err(Error::InvalidData(
                "node network differs from configured network".into(),
            ));
        }
        if tip.blocks < 0 {
            return Err(Error::InvalidData("node returned a negative height".into()));
        }
        decode_hash(&tip.bestblockhash)?;
        Ok(tip)
    }

    pub async fn fetch_block_hash(&self, height: i64) -> Result<Hash> {
        let value = self.call_rpc("getblockhash", json!([height])).await?;
        let text = value
            .as_str()
            .ok_or_else(|| Error::InvalidData("getblockhash did not return a hash".into()))?;
        decode_hash(text)
    }

    pub async fn fetch_block(&self, hash: &Hash) -> Result<Block> {
        let value = self
            .call_rpc("getblock", json!([hex::encode(hash), 2]))
            .await?;
        let block = decode_block(&value.to_string())?;
        if block.hash != *hash {
            return Err(Error::InvalidData(
                "getblock returned a different block hash".into(),
            ));
        }
        Ok(block)
    }

    pub async fn fetch_mempool_ids(&self) -> Result<MempoolIds> {
        let value = self.call_rpc("getrawmempool", json!([false, true])).await?;
        let mut snapshot: MempoolIds = serde_json::from_value(value)
            .map_err(|_| Error::InvalidData("invalid mempool sequence response".into()))?;
        if snapshot.txids.len() > 100_000 {
            return Err(Error::InvalidData(
                "mempool exceeds the current 100,000 transaction limit".into(),
            ));
        }
        for txid in &snapshot.txids {
            decode_hash(txid)?;
        }
        snapshot.txids.sort();
        Ok(snapshot)
    }

    pub async fn fetch_pending_transaction(&self, txid: &Hash) -> Result<Transaction> {
        let value = self
            .call_rpc("getrawtransaction", json!([hex::encode(txid), true]))
            .await?;
        let transaction = decode_transaction(&value.to_string())?;
        if transaction.txid != *txid || transaction.is_coinbase {
            return Err(Error::InvalidData(
                "unexpected pending transaction ID or coinbase".into(),
            ));
        }
        Ok(transaction)
    }

    pub(crate) async fn call_rpc(&self, method: &str, params: Value) -> Result<Value> {
        let result = self.call_rpc_allow_null(method, params).await?;
        if result.is_null() {
            return Err(Error::InvalidData(format!(
                "{method} returned a null result"
            )));
        }
        Ok(result)
    }

    // submitblock reports successful validation with a null result.
    pub(crate) async fn call_rpc_allow_null(&self, method: &str, params: Value) -> Result<Value> {
        // Read for every call so daemon cookie rotation does not require restart.
        let cookie = tokio::fs::read_to_string(&self.cookie_path).await?;
        let (username, password) = cookie
            .trim_end()
            .split_once(':')
            .ok_or_else(|| Error::InvalidData("RPC cookie has an invalid format".into()))?;
        let mut response = self
            .http
            .post(self.url.clone())
            .basic_auth(username, Some(password))
            .json(
                &json!({"jsonrpc": "2.0", "id": "northbound", "method": method, "params": params}),
            )
            .send()
            .await
            .map_err(|_| {
                Error::Transport(format!("{method} request failed; check node availability"))
            })?;
        if !response.status().is_success() {
            return Err(Error::Transport(format!(
                "{method} returned HTTP {}",
                response.status()
            )));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::Transport(format!("{method} response interrupted")))?
        {
            if chunk.len() > MAX_RPC_RESPONSE_BYTES - bytes.len() {
                return Err(Error::InvalidData("RPC response exceeded 64 MiB".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut response: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Error::InvalidData(format!("{method} returned invalid JSON")))?;
        if response["id"] != "northbound" || response["jsonrpc"] != "2.0" {
            return Err(Error::InvalidData(
                "RPC response ID or version does not match the request".into(),
            ));
        }
        if !response["error"].is_null() {
            let code = response["error"]["code"]
                .as_i64()
                .ok_or_else(|| Error::InvalidData("RPC error has no numeric code".into()))?;
            return Err(Error::Remote {
                method: method.into(),
                code,
            });
        }
        let result = response
            .get_mut("result")
            .ok_or_else(|| Error::InvalidData(format!("{method} response has no result")))?;
        Ok(result.take())
    }
}
