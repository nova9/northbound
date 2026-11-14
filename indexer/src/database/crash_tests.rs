use std::env;
use std::path::PathBuf;
use std::process::Command;

use crate::{IndexDatabase, decode_block};
use rusqlite::{Connection, params};
use serde_json::json;

const GENESIS: [u8; 32] = [1; 32];
const TRANSACTION_ID: [u8; 32] = [2; 32];

#[test]
fn abrupt_exit_before_and_after_commit_preserves_atomic_progress() {
    for mode in ["before", "after"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chain.sqlite");
        let output = Command::new(env::current_exe().unwrap())
            .args(["--exact", "database::crash_tests::crash_worker"])
            .env("INDEXER_CRASH_TEST_PATH", &path)
            .env("INDEXER_CRASH_TEST_MODE", mode)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(91),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reopened = IndexDatabase::open(&path, "regtest", GENESIS).unwrap();
        let reader = Connection::open(&path).unwrap();
        let expected_count = if mode == "before" { 0 } else { 1 };
        for table in ["blocks", "transactions", "outputs"] {
            let count: i64 = reader
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, expected_count, "{mode} commit: {table}");
        }
        assert_eq!(reopened.tip().unwrap().is_some(), mode == "after");
        let integrity: String = reader
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }
}

#[test]
fn crash_worker() {
    let Some(path) = env::var_os("INDEXER_CRASH_TEST_PATH") else {
        return;
    };
    let path = PathBuf::from(path);
    let mut database = IndexDatabase::open(&path, "regtest", GENESIS).unwrap();
    if env::var("INDEXER_CRASH_TEST_MODE").unwrap() == "after" {
        let block = decode_block(
            &json!({
                "hash": hex::encode(GENESIS), "height": 0, "time": 1000,
                "tx": [{"txid": hex::encode(TRANSACTION_ID), "vin": [{"coinbase": "00"}],
                    "vout": [{"n": 0, "value": 512, "scriptPubKey": {"hex": "51"}}]}]
            })
            .to_string(),
        )
        .unwrap();
        database.apply_block(&block).unwrap();
    } else {
        // Stage related records without committing, then exit without running
        // Rust destructors. Reopening must recover from SQLite's WAL alone.
        let mut writer = Connection::open(&path).unwrap();
        writer.pragma_update(None, "foreign_keys", true).unwrap();
        let pending = writer.transaction().unwrap();
        pending.execute("INSERT INTO blocks (hash, height, previous_hash, timestamp) VALUES (?1, 0, NULL, 1000)",
            [GENESIS.as_slice()]).unwrap();
        pending.execute("INSERT INTO transactions (txid, block_hash, position, is_coinbase) VALUES (?1, ?2, 0, 1)",
            params![TRANSACTION_ID.as_slice(), GENESIS.as_slice()]).unwrap();
        pending.execute("INSERT INTO outputs (txid, vout, amount, script, utxo_eligible) VALUES (?1, 0, 51200000000, X'51', 0)",
            [TRANSACTION_ID.as_slice()]).unwrap();
        std::process::exit(91);
    }
    std::process::exit(91);
}
