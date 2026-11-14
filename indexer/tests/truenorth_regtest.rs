#[path = "truenorth/utxo_scan.rs"]
mod utxo_scan;

use std::env;
use std::path::PathBuf;
use std::process::Command;

use indexer::{IndexDatabase, SyncProgress, synchronize_next_block};
use rusqlite::{Connection, params};
use serde_json::Value;
use truenorth_rpc::NodeClient;

fn run_cli(arguments: &[&str]) -> String {
    assert_eq!(
        env::var("TRUENORTH_TEST_ISOLATED").as_deref(),
        Ok("1"),
        "run scripts/test-truenorth-integration.sh indexer; never invalidate a user's node"
    );
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
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn cli_json(arguments: &[&str]) -> Value {
    serde_json::from_str(&run_cli(arguments)).unwrap()
}

async fn catch_up(node: &NodeClient, database: &mut IndexDatabase) -> (usize, usize) {
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

fn confirmed_dump(connection: &Connection) -> Vec<String> {
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

#[tokio::test]
#[ignore = "requires an isolated TrueNorth regtest node; use scripts/test-truenorth-integration.sh indexer"]
async fn indexes_resumes_rolls_back_and_rebuilds_against_core() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("index.sqlite");
    let url = format!(
        "http://127.0.0.1:{}",
        env::var("TRUENORTH_TEST_RPC_PORT").unwrap()
    );
    let cookie =
        PathBuf::from(env::var_os("TRUENORTH_TEST_DATA_DIR").unwrap()).join("regtest/.cookie");
    let node = NodeClient::new(&url, cookie.clone(), "regtest").unwrap();
    // Verify isolation before any mining or invalidation.
    assert_eq!(env::var("TRUENORTH_TEST_ISOLATED").as_deref(), Ok("1"));
    assert_eq!(node.fetch_tip().await.unwrap().blocks, 0);
    let release = cli_json(&["getnetworkinfo"]);
    println!("Testing {}", release["subversion"]);
    assert!(
        release["subversion"]
            .as_str()
            .unwrap()
            .contains(include_str!("../../TRUENORTH_VERSION").trim())
    );
    let genesis = node.fetch_block_hash(0).await.unwrap();
    let mut database = IndexDatabase::open(&path, "regtest", genesis).unwrap();
    assert_eq!(catch_up(&node, &mut database).await, (1, 0));
    let reader = Connection::open(&path).unwrap();
    let genesis_eligible: bool = reader
        .query_row("SELECT utxo_eligible FROM outputs", [], |row| row.get(0))
        .unwrap();
    assert!(!genesis_eligible);

    let unused_address = run_cli(&["getnewaddress"]);
    assert_eq!(
        utxo_scan::assert_matches_scan(
            &node,
            &mut database,
            &path,
            std::slice::from_ref(&unused_address)
        )
        .await,
        0
    );

    let mining_address = run_cli(&["getnewaddress"]);
    run_cli(&["generatetoaddress", "101", &mining_address]);
    assert_eq!(catch_up(&node, &mut database).await, (101, 0));
    let mining_addresses = vec![mining_address.clone(), unused_address.clone()];
    assert_eq!(
        utxo_scan::assert_matches_scan(&node, &mut database, &path, &mining_addresses).await,
        0
    );
    let receiving_address = run_cli(&["getnewaddress"]);
    let address_info = cli_json(&["validateaddress", &receiving_address]);
    let script = hex::decode(address_info["scriptPubKey"].as_str().unwrap()).unwrap();
    assert_eq!(&script[..2], &[0x52, 0x20]);
    let payment = run_cli(&[
        "-named",
        "sendtoaddress",
        &format!("address={receiving_address}"),
        "amount=1",
        "fee_rate=1",
    ]);
    let mined = cli_json(&["generatetoaddress", "1", &mining_address]);
    let original_block = mined[0].as_str().unwrap();
    assert_eq!(catch_up(&node, &mut database).await, (1, 0));
    let decoded_payment = cli_json(&["getrawtransaction", &payment, "true", original_block]);
    let mut change_address = None;
    for output in decoded_payment["vout"].as_array().unwrap() {
        if let Some(address) = output["scriptPubKey"]["address"].as_str()
            && address != receiving_address
        {
            change_address = Some(address.to_owned());
        }
    }
    let requested_addresses = vec![
        mining_address.clone(),
        receiving_address.clone(),
        change_address.expect("payment should include change"),
        unused_address,
        receiving_address.clone(),
    ];
    assert_eq!(
        utxo_scan::assert_matches_scan(&node, &mut database, &path, &requested_addresses).await,
        0
    );

    let payment_id = hex::decode(&payment).unwrap();
    let (vout, amount, saved_script): (u32, i64, Vec<u8>) = reader
        .query_row(
            "SELECT vout, amount, script FROM outputs WHERE txid = ?1 AND script = ?2",
            params![payment_id, script],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(amount, 100_000_000);
    let utxo = cli_json(&["gettxout", &payment, &vout.to_string(), "false"]);
    assert_eq!(utxo["scriptPubKey"]["hex"], hex::encode(saved_script));
    let input: (Vec<u8>, u32) = reader
        .query_row(
            "SELECT previous_txid, previous_vout FROM spends WHERE spending_txid = ?1",
            [payment_id.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(
        run_cli(&[
            "gettxout",
            &hex::encode(&input.0),
            &input.1.to_string(),
            "false"
        ])
        .is_empty()
    );

    drop(database);
    database = IndexDatabase::open(&path, "regtest", genesis).unwrap();
    assert_eq!(catch_up(&node, &mut database).await, (0, 0));
    assert_eq!(
        utxo_scan::assert_matches_scan(&node, &mut database, &path, &requested_addresses).await,
        0
    );

    // Replace height 102 with an explicitly empty block, leaving payment pending.
    run_cli(&["invalidateblock", original_block]);
    let replacement = cli_json(&["generateblock", &mining_address, "[]"]);
    assert_ne!(replacement["hash"].as_str().unwrap(), original_block);
    assert_eq!(catch_up(&node, &mut database).await, (1, 1));
    assert_eq!(
        utxo_scan::assert_matches_scan(&node, &mut database, &path, &requested_addresses).await,
        1
    );
    let payment_rows: i64 = reader
        .query_row(
            "SELECT count(*) FROM transactions WHERE txid = ?1",
            [payment_id.as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(payment_rows, 0);
    assert!(
        !cli_json(&[
            "gettxout",
            &hex::encode(&input.0),
            &input.1.to_string(),
            "false"
        ])
        .is_null()
    );
    let restored: i64 = reader.query_row(
        "SELECT count(*) FROM outputs WHERE txid = ?1 AND vout = ?2
         AND NOT EXISTS (SELECT 1 FROM spends WHERE previous_txid = outputs.txid AND previous_vout = outputs.vout)",
        params![input.0, input.1], |row| row.get(0),
    ).unwrap();
    assert_eq!(restored, 1);

    // Mining includes the pending payment on the replacement branch exactly once.
    run_cli(&["generatetoaddress", "3", &mining_address]);
    assert_eq!(catch_up(&node, &mut database).await, (3, 0));
    assert_eq!(
        utxo_scan::assert_matches_scan(&node, &mut database, &path, &requested_addresses).await,
        0
    );
    let payment_rows: i64 = reader
        .query_row(
            "SELECT count(*) FROM transactions WHERE txid = ?1",
            [payment_id.as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(payment_rows, 1);

    // No shallow rollback cap: remove more than fifty blocks, then restore them.
    let ancestor = run_cli(&["getblockhash", "50"]);
    run_cli(&["invalidateblock", &ancestor]);
    let (_, removed) = catch_up(&node, &mut database).await;
    assert!(removed > 50);
    assert_eq!(database.tip().unwrap().unwrap().height, 49);
    utxo_scan::assert_matches_scan(&node, &mut database, &path, &requested_addresses).await;
    run_cli(&["reconsiderblock", &ancestor]);
    catch_up(&node, &mut database).await;

    utxo_scan::assert_matches_scan(&node, &mut database, &path, &requested_addresses).await;

    let mut rebuilt =
        IndexDatabase::open(&directory.path().join("rebuilt.sqlite"), "regtest", genesis).unwrap();
    catch_up(&node, &mut rebuilt).await;
    let rebuilt_reader = Connection::open(directory.path().join("rebuilt.sqlite")).unwrap();
    assert_eq!(confirmed_dump(&reader), confirmed_dump(&rebuilt_reader));
    assert_eq!(database.tip().unwrap(), rebuilt.tip().unwrap());

    let before = database.tip().unwrap();
    let unavailable = NodeClient::new("http://127.0.0.1:1", cookie.clone(), "regtest").unwrap();
    assert!(
        synchronize_next_block(&unavailable, &mut database)
            .await
            .is_err()
    );
    assert_eq!(database.tip().unwrap(), before);
    let wrong_network = NodeClient::new(&url, cookie.clone(), "main").unwrap();
    assert!(
        synchronize_next_block(&wrong_network, &mut database)
            .await
            .is_err()
    );
    assert_eq!(database.tip().unwrap(), before);

    // Exercise the executable too, using a separate database from the library.
    let binary_path = directory.path().join("binary.sqlite");
    for first_run in [true, false] {
        let output = Command::new(env!("CARGO_BIN_EXE_indexer"))
            .arg("--once")
            .env("TRUENORTH_NETWORK", "regtest")
            .env("TRUENORTH_RPC_URL", &url)
            .env("TRUENORTH_RPC_COOKIE_PATH", &cookie)
            .env("TRUENORTH_INDEX_DATABASE", &binary_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Caught up at"));
        assert_eq!(stdout.contains("Indexed 0 "), first_run);
    }
}
