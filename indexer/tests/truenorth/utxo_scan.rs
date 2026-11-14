use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use indexer::{IndexDatabase, IndexReader, MempoolCache, decode_addresses, refresh_mempool};
use serde::Deserialize;
use serde_json::{Number, json};
use shared::WalletUtxo;
use truenorth_rpc::NodeClient;

use super::{cli_json, run_cli};

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
pub async fn assert_matches_scan(
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
    for output in snapshot.query_utxos().unwrap() {
        let outpoint = (output.transaction_id.clone(), output.output_index);
        assert!(
            actual.insert(outpoint, output).is_none(),
            "duplicate indexed outpoint"
        );
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
