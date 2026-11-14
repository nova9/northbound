use std::path::PathBuf;

use crate::{Block, Hash, IndexDatabase, decode_block};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use tempfile::TempDir;

pub fn hash(number: u64) -> Hash {
    let mut hash = [0; 32];
    hash[24..].copy_from_slice(&number.to_be_bytes());
    hash
}

pub fn coinbase(txid: u64, script: &str) -> Value {
    json!({
        "txid": hex::encode(hash(txid)), "vin": [{"coinbase": "00"}],
        "vout": [{"n": 0, "value": 512, "scriptPubKey": {"hex": script}}]
    })
}

pub fn payment(txid: u64, previous_txid: u64, amount: &str) -> Value {
    let amount: serde_json::Number = serde_json::from_str(amount).unwrap();
    json!({
        "txid": hex::encode(hash(txid)),
        "vin": [{"txid": hex::encode(hash(previous_txid)), "vout": 0}],
        "vout": [{"n": 0, "value": amount, "scriptPubKey": {"hex": "51"}}]
    })
}

pub fn block(height: u64, branch: u64, mut extra: Vec<Value>) -> Block {
    let block_hash = hash(height + 1 + branch * 10_000);
    let parent = if height == 0 {
        None
    } else {
        Some(hex::encode(hash(height)))
    };
    let mut transactions = vec![coinbase(1000 + height + branch * 10_000, "51")];
    transactions.append(&mut extra);
    decode_block(
        &json!({
            "hash": hex::encode(block_hash), "height": height,
            "previousblockhash": parent, "time": 1000 + height, "tx": transactions
        })
        .to_string(),
    )
    .unwrap()
}

pub struct Fixture {
    pub directory: TempDir,
    pub database: IndexDatabase,
}

impl Fixture {
    pub fn new(height: u64) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut database =
            IndexDatabase::open(&directory.path().join("chain.sqlite"), "regtest", hash(1))
                .unwrap();
        for height in 0..=height {
            database.apply_block(&block(height, 0, vec![])).unwrap();
        }
        Self {
            directory,
            database,
        }
    }

    pub fn path(&self) -> PathBuf {
        self.directory.path().join("chain.sqlite")
    }

    pub fn reader(&self) -> Connection {
        Connection::open_with_flags(self.path(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
    }

    pub fn count(&self, table: &str) -> i64 {
        self.reader()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }
}
