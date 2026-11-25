mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};

use indexer::{
    IndexDatabase, IndexReader, MempoolCache, decode_addresses, refresh_mempool,
    synchronize_next_block,
};
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::{Number, json};
use shared::WalletUtxo;
use truenorth_rpc::NodeClient;

use support::{catch_up, cli_json, confirmed_dump, run_cli};

fn test_node_client() -> (NodeClient, String, PathBuf) {
    assert_eq!(env::var("TRUENORTH_TEST_ISOLATED").as_deref(), Ok("1"));
    let url = format!(
        "http://127.0.0.1:{}",
        env::var("TRUENORTH_TEST_RPC_PORT").unwrap()
    );
    let cookie =
        PathBuf::from(env::var_os("TRUENORTH_TEST_DATA_DIR").unwrap()).join("regtest/.cookie");
    let node = NodeClient::new(&url, cookie.clone(), "regtest").unwrap();
    (node, url, cookie)
}

fn ensure_spendable_funds() -> String {
    let mining_address = run_cli(&["getnewaddress"]);
    let height = cli_json(&["getblockchaininfo"])["blocks"].as_u64().unwrap();
    if height < 101 {
        let needed = 101 - height;
        run_cli(&["generatetoaddress", &needed.to_string(), &mining_address]);
    }
    mining_address
}

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh indexer"]
async fn indexes_blocks_and_matches_scantxoutset_across_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("index.sqlite");
    let (node, _, _) = test_node_client();

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
    catch_up(&node, &mut database).await;

    let reader = Connection::open(&path).unwrap();
    let genesis_eligible: bool = reader
        .query_row(
            "SELECT o.utxo_eligible
             FROM outputs o
             JOIN transactions t ON t.txid = o.txid
             JOIN blocks b ON b.hash = t.block_hash
             WHERE b.height = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!genesis_eligible);

    let unused_address = run_cli(&["getnewaddress"]);
    assert_eq!(
        assert_matches_scan(
            &node,
            &mut database,
            &path,
            std::slice::from_ref(&unused_address)
        )
        .await,
        0
    );

    let mining_address = ensure_spendable_funds();
    catch_up(&node, &mut database).await;
    let mining_addresses = vec![mining_address.clone(), unused_address.clone()];
    assert_eq!(
        assert_matches_scan(&node, &mut database, &path, &mining_addresses).await,
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
    let payment_block = mined[0].as_str().unwrap();
    assert_eq!(catch_up(&node, &mut database).await, (1, 0));

    let decoded_payment = cli_json(&["getrawtransaction", &payment, "true", payment_block]);
    let mut change_address = None;
    for output in decoded_payment["vout"].as_array().unwrap() {
        if let Some(address) = output["scriptPubKey"]["address"].as_str()
            && address != receiving_address
        {
            change_address = Some(address.to_owned());
        }
    }
    let requested_addresses = vec![
        mining_address,
        receiving_address.clone(),
        change_address.expect("payment should include change"),
        unused_address,
        receiving_address,
    ];
    assert_eq!(
        assert_matches_scan(&node, &mut database, &path, &requested_addresses).await,
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

    // Reopening an existing database resumes cleanly without re-applying blocks.
    drop(database);
    database = IndexDatabase::open(&path, "regtest", genesis).unwrap();
    assert_eq!(catch_up(&node, &mut database).await, (0, 0));
    assert_eq!(
        assert_matches_scan(&node, &mut database, &path, &requested_addresses).await,
        0
    );
}

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh indexer"]
async fn rolls_back_single_block_and_deep_reorg_and_matches_rebuild() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("index.sqlite");
    let (node, _, _) = test_node_client();

    let mining_address = ensure_spendable_funds();
    let genesis = node.fetch_block_hash(0).await.unwrap();
    let mut database = IndexDatabase::open(&path, "regtest", genesis).unwrap();
    catch_up(&node, &mut database).await;
    let reader = Connection::open(&path).unwrap();

    let receiving_address = run_cli(&["getnewaddress"]);
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
    let payment_id = hex::decode(&payment).unwrap();
    let input: (Vec<u8>, u32) = reader
        .query_row(
            "SELECT previous_txid, previous_vout FROM spends WHERE spending_txid = ?1",
            [payment_id.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let spent_script: Vec<u8> = reader
        .query_row(
            "SELECT script FROM outputs WHERE txid = ?1 AND vout = ?2",
            params![input.0, input.1],
            |row| row.get(0),
        )
        .unwrap();
    let spent_address = indexer::address_from_script(&spent_script, "regtest").unwrap();
    let requested_addresses = vec![
        mining_address.clone(),
        spent_address,
        receiving_address,
        change_address.expect("payment should include change"),
    ];

    // Replace the payment block with an explicitly empty block, leaving payment pending.
    run_cli(&["invalidateblock", original_block]);
    let replacement = cli_json(&["generateblock", &mining_address, "[]"]);
    assert_ne!(replacement["hash"].as_str().unwrap(), original_block);
    assert_eq!(catch_up(&node, &mut database).await, (1, 1));
    assert_eq!(
        assert_matches_scan(&node, &mut database, &path, &requested_addresses).await,
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
    let restored: i64 = reader
        .query_row(
            "SELECT count(*) FROM outputs WHERE txid = ?1 AND vout = ?2
             AND NOT EXISTS (SELECT 1 FROM spends WHERE previous_txid = outputs.txid AND previous_vout = outputs.vout)",
            params![input.0, input.1],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(restored, 1);

    // Mining includes the pending payment on the replacement branch exactly once.
    run_cli(&["generatetoaddress", "3", &mining_address]);
    assert_eq!(catch_up(&node, &mut database).await, (3, 0));
    assert_eq!(
        assert_matches_scan(&node, &mut database, &path, &requested_addresses).await,
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
    assert_matches_scan(&node, &mut database, &path, &requested_addresses).await;
    run_cli(&["reconsiderblock", &ancestor]);
    catch_up(&node, &mut database).await;
    assert_matches_scan(&node, &mut database, &path, &requested_addresses).await;

    let mut rebuilt =
        IndexDatabase::open(&directory.path().join("rebuilt.sqlite"), "regtest", genesis).unwrap();
    catch_up(&node, &mut rebuilt).await;
    let rebuilt_reader = Connection::open(directory.path().join("rebuilt.sqlite")).unwrap();
    assert_eq!(confirmed_dump(&reader), confirmed_dump(&rebuilt_reader));
    assert_eq!(database.tip().unwrap(), rebuilt.tip().unwrap());
}

#[tokio::test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh indexer"]
async fn rejects_unavailable_node_and_wrong_network_without_moving_tip() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("index.sqlite");
    let (node, url, cookie) = test_node_client();

    let genesis = node.fetch_block_hash(0).await.unwrap();
    let mut database = IndexDatabase::open(&path, "regtest", genesis).unwrap();
    catch_up(&node, &mut database).await;

    let before = database.tip().unwrap();
    let unavailable = NodeClient::new("http://127.0.0.1:1", cookie.clone(), "regtest").unwrap();
    assert!(
        synchronize_next_block(&unavailable, &mut database)
            .await
            .is_err()
    );
    assert_eq!(database.tip().unwrap(), before);

    let wrong_network = NodeClient::new(&url, cookie, "main").unwrap();
    assert!(
        synchronize_next_block(&wrong_network, &mut database)
            .await
            .is_err()
    );
    assert_eq!(database.tip().unwrap(), before);
}

#[derive(Deserialize)]
struct ScanResult {
    success: bool,
    height: u64,
    bestblock: String,
    unspents: Vec<ScanOutput>,
    total_amount: Number,
}

#[derive(Deserialize)]
struct ScanOutput {
    txid: String,
    vout: u32,
    amount: Number,
    #[serde(rename = "scriptPubKey")]
    script: String,
    height: u64,
    confirmations: u64,
    coinbase: bool,
}

/// Compare every UTXO field with the daemon, accounting for the deliberate
/// exclusion of pending spends. Returns the number excluded from the raw scan.
async fn assert_matches_scan(
    node: &NodeClient,
    database: &mut IndexDatabase,
    path: &Path,
    addresses: &[String],
) -> usize {
    refresh_mempool(node, database, &mut MempoolCache::new())
        .await
        .unwrap();
    let before = node.fetch_mempool_ids().await.unwrap();
    let mut pending_spends = BTreeSet::new();
    for id in &before.txids {
        let transaction = cli_json(&["getrawtransaction", id, "true"]);
        for input in transaction["vin"].as_array().unwrap() {
            pending_spends.insert((
                input["txid"].as_str().unwrap().to_owned(),
                u32::try_from(input["vout"].as_u64().unwrap()).unwrap(),
            ));
        }
    }
    let mut descriptors = Vec::new();
    for address in addresses {
        descriptors.push(format!("addr({address})"));
    }
    // Historical scans belong only in this isolated test, never the API path.
    let result = run_cli(&["scantxoutset", "start", &json!(descriptors).to_string()]);
    let scan: ScanResult = serde_json::from_str(&result).unwrap();
    assert!(scan.success, "daemon UTXO scan did not complete");
    assert_eq!(
        node.fetch_mempool_ids().await.unwrap(),
        before,
        "mempool changed during comparison"
    );
    let tip = node.fetch_tip().await.unwrap();
    assert_eq!(tip.blocks as u64, scan.height);
    assert_eq!(tip.bestblockhash, scan.bestblock);

    let scripts = decode_addresses(addresses, "regtest").unwrap();
    let mut reader = IndexReader::open(path, "regtest").unwrap();
    let snapshot = reader.read_snapshot(&scripts).unwrap();
    assert_eq!(snapshot.status.indexed_height, Some(scan.height));
    assert_eq!(
        snapshot.status.indexed_hash.as_deref(),
        Some(scan.bestblock.as_str())
    );
    let mut expected = BTreeMap::new();
    let mut scan_total = 0_u64;
    let mut excluded = 0;
    for output in scan.unspents {
        let amount = scan_atomic_units(&output.amount);
        scan_total = scan_total.checked_add(amount).unwrap();
        let outpoint = (output.txid.to_ascii_lowercase(), output.vout);
        // scantxoutset ignores the mempool. Obtain pending inputs independently
        // from the daemon instead of using the indexer's own spend records.
        if pending_spends.contains(&outpoint) {
            excluded += 1;
            continue;
        }
        let utxo = WalletUtxo {
            transaction_id: outpoint.0.clone(),
            output_index: output.vout,
            amount,
            script_pubkey: output.script.to_ascii_lowercase(),
            block_height: output.height,
            confirmations: output.confirmations,
            coinbase: output.coinbase,
        };
        assert!(
            expected.insert(outpoint, utxo).is_none(),
            "duplicate scan outpoint"
        );
    }
    assert_eq!(scan_total, scan_atomic_units(&scan.total_amount));
    let mut actual = BTreeMap::new();
    let mut cursor = None;
    loop {
        let page = snapshot.query_utxos(cursor.as_deref(), 17).unwrap();
        assert!(page.utxos.len() <= 17);
        for output in page.utxos {
            let outpoint = (output.transaction_id.clone(), output.output_index);
            assert!(
                actual.insert(outpoint, output).is_none(),
                "duplicate indexed outpoint"
            );
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        actual, expected,
        "indexed UTXOs differ from scantxoutset at height {}",
        scan.height
    );
    println!(
        "scantxoutset matches {} indexed UTXOs at height {} ({excluded} pending spends excluded)",
        actual.len(),
        scan.height
    );
    excluded
}

fn scan_atomic_units(number: &Number) -> u64 {
    // Core's ValueFromAmount emits fixed decimal amounts. Keep this conversion
    // independent of the production decoder so a shared bug cannot hide a mismatch.
    let text = number.to_string();
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    assert!(fraction.len() <= 8, "scan returned sub-atomic precision");
    let whole = whole
        .parse::<u64>()
        .unwrap()
        .checked_mul(100_000_000)
        .unwrap();
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()
            .unwrap()
            .checked_mul(10_u64.pow(8 - fraction.len() as u32))
            .unwrap()
    };
    whole.checked_add(fraction).unwrap()
}
