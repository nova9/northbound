use std::{env, process::Command};

use indexer::{IndexDatabase, SyncProgress, synchronize_next_block};
use rusqlite::Connection;
use serde_json::Value;
use truenorth_rpc::NodeClient;

pub fn run_cli(arguments: &[&str]) -> String {
    assert_eq!(
        env::var("TRUENORTH_TEST_ISOLATED").as_deref(),
        Ok("1"),
        "run scripts/test-truenorth-integration.sh indexer; never invalidate a user's node"
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

pub async fn catch_up(node: &NodeClient, database: &mut IndexDatabase) -> (usize, usize) {
    let mut applied = 0;
    let mut removed = 0;
    for _ in 0..2000 {
        match synchronize_next_block(node, database).await.unwrap() {
            SyncProgress::Applied(_) => applied += 1,
            SyncProgress::RolledBack(_) => removed += 1,
            SyncProgress::CaughtUp(_) => return (applied, removed),
            SyncProgress::ChainChanged => {}
        }
    }
    panic!("indexer did not converge");
}

pub fn confirmed_dump(connection: &Connection) -> Vec<String> {
    let mut dump = Vec::new();
    for query in [
        "SELECT hex(hash), height, hex(previous_hash), timestamp FROM blocks ORDER BY height",
        "SELECT hex(txid), hex(block_hash), position, is_coinbase FROM transactions ORDER BY txid",
        "SELECT hex(txid), vout, amount, hex(script), utxo_eligible FROM outputs ORDER BY txid, vout",
        "SELECT hex(spending_txid), vin, hex(previous_txid), previous_vout FROM spends ORDER BY spending_txid, vin",
    ] {
        let mut statement = connection.prepare(query).unwrap();
        let column_count = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                let mut fields = Vec::new();
                for column in 0..column_count {
                    fields.push(format!("{:?}", row.get_ref(column)?));
                }
                Ok(fields.join("|"))
            })
            .unwrap();
        for row in rows {
            dump.push(row.unwrap());
        }
    }
    dump
}
