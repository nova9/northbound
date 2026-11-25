use crate::test_support::{Fixture, block, hash, payment};
use crate::{Error, IndexReader, ScriptAddress};
use rusqlite::Connection;
use serde_json::json;

fn scripts() -> Vec<ScriptAddress> {
    vec![
        ScriptAddress {
            address: "test-miner".into(),
            script: vec![0x51],
        },
        ScriptAddress {
            address: "test-change".into(),
            script: vec![0x52],
        },
    ]
}

fn ready(fixture: &mut Fixture, sequence: u64) {
    let base = fixture.database.tip().unwrap().unwrap();
    fixture
        .database
        .publish_mempool_snapshot(&base, sequence, &[])
        .unwrap();
}

#[test]
fn transaction_status_survives_confirmation_and_rollback_without_outputs_or_history() {
    let mut fixture = Fixture::new(100);
    ready(&mut fixture, 1);
    let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
    let id = hash(2000);
    let unknown = reader
        .read_snapshot(&[])
        .unwrap()
        .query_transaction_status(&id)
        .unwrap();
    assert!(!unknown.known);
    assert_eq!(unknown.block_height, None);
    let payment =
        truenorth_rpc::decode_transaction(&payment(2000, 1001, "511.99999837").to_string())
            .unwrap();
    let base = fixture.database.tip().unwrap().unwrap();
    fixture
        .database
        .publish_mempool_snapshot(&base, 2, std::slice::from_ref(&payment))
        .unwrap();
    let pending_snapshot = reader.read_snapshot(&[]).unwrap();
    let pending = pending_snapshot.query_transaction_status(&id).unwrap();
    assert!(pending.known);
    assert_eq!(pending.block_height, None);
    let mut confirmed_block = block(101, 0, vec![]);
    confirmed_block.transactions.push(payment.clone());
    fixture.database.apply_block(&confirmed_block).unwrap();
    assert_eq!(
        pending_snapshot
            .query_transaction_status(&id)
            .unwrap()
            .block_height,
        None
    );
    drop(pending_snapshot);
    ready(&mut fixture, 3);
    let confirmed = reader
        .read_snapshot(&[])
        .unwrap()
        .query_transaction_status(&id)
        .unwrap();
    assert!(confirmed.known);
    assert_eq!(confirmed.block_height, Some(101));
    let tip = fixture.database.tip().unwrap().unwrap();
    fixture.database.rollback_tip(&tip).unwrap();
    assert!(matches!(
        reader.read_snapshot(&[]),
        Err(Error::Unavailable(_))
    ));
    fixture
        .database
        .publish_mempool_snapshot(&base, 4, &[payment])
        .unwrap();
    let pending = reader
        .read_snapshot(&[])
        .unwrap()
        .query_transaction_status(&id)
        .unwrap();
    assert!(pending.known);
    assert_eq!(pending.block_height, None);
    ready(&mut fixture, 5);
    assert!(
        !reader
            .read_snapshot(&[])
            .unwrap()
            .query_transaction_status(&id)
            .unwrap()
            .known
    );
}

#[test]
fn confirmed_queries_preserve_change_deduplicate_and_paginate() {
    let mut fixture = Fixture::new(100);
    let mut payment = payment(2000, 1001, "1");
    payment["vout"][0]["scriptPubKey"]["hex"] = json!("53");
    payment["vout"].as_array_mut().unwrap().push(json!({
        "n": 1, "value": serde_json::from_str::<serde_json::Number>("510.99999837").unwrap(),
        "scriptPubKey": {"hex": "52"}
    }));
    fixture
        .database
        .apply_block(&block(101, 0, vec![payment]))
        .unwrap();
    ready(&mut fixture, 1);
    assert!(fixture.count("blocks") > 0);
    let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
    let mut requested = scripts();
    requested.push(requested[0].clone());
    let snapshot = reader.read_snapshot(&requested).unwrap();
    let utxos = snapshot
        .query_utxos(None, shared::MAX_UTXO_PAGE_SIZE)
        .unwrap()
        .utxos;
    assert!(
        !utxos
            .iter()
            .any(|output| output.transaction_id == hex::encode(hash(1001)))
    );
    assert!(utxos.iter().any(|output| output.amount == 51_099_999_837));
    assert!(
        utxos
            .iter()
            .any(|output| output.coinbase && output.confirmations == 1)
    );
    assert_eq!(
        snapshot.query_script_activity().unwrap(),
        vec!["test-change", "test-miner"]
    );
    let first = snapshot.query_transaction_history(None, 2).unwrap();
    assert!(first.next_cursor.is_some());
    let outgoing = first
        .transactions
        .iter()
        .find(|tx| tx.transaction_id == hex::encode(hash(2000)))
        .unwrap();
    assert_eq!(outgoing.net_amount, -100_000_163);
    assert_eq!(outgoing.network_fee, Some(163));
    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot
            .query_transaction_history(cursor.as_deref(), 7)
            .unwrap();
        for transaction in page.transactions {
            ids.push(transaction.transaction_id);
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let count = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count);
    assert_eq!(count, 103);
    let cursor = first.next_cursor.unwrap();
    drop(snapshot);
    ready(&mut fixture, 2);
    let snapshot = reader.read_snapshot(&requested).unwrap();
    assert!(matches!(
        snapshot.query_transaction_history(Some(&cursor), 2),
        Err(Error::CursorChanged)
    ));
}

#[test]
fn pending_parent_child_eviction_and_failed_publication_are_coherent() {
    let mut fixture = Fixture::new(100);
    let base = fixture.database.tip().unwrap().unwrap();
    let parent =
        truenorth_rpc::decode_transaction(&payment(2000, 1001, "511.99999837").to_string())
            .unwrap();
    let child = truenorth_rpc::decode_transaction(&payment(2001, 2000, "511.99999674").to_string())
        .unwrap();
    fixture
        .database
        .publish_mempool_snapshot(&base, 2, &[parent.clone(), child])
        .unwrap();
    let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
    let requested = scripts();
    let snapshot = reader.read_snapshot(&requested).unwrap();
    assert!(
        !snapshot
            .query_utxos(None, shared::MAX_UTXO_PAGE_SIZE)
            .unwrap()
            .utxos
            .iter()
            .any(|output| output.transaction_id == hex::encode(hash(1001)))
    );
    let history = snapshot.query_transaction_history(None, 10).unwrap();
    assert_eq!(
        history
            .transactions
            .iter()
            .filter(|tx| tx.confirmations == 0)
            .count(),
        2
    );
    let cursor = history.next_cursor.unwrap();
    drop(snapshot);
    let mut invalid = parent.clone();
    invalid.txid = hash(2002);
    assert!(
        fixture
            .database
            .publish_mempool_snapshot(&base, 3, &[parent, invalid])
            .is_err()
    );
    assert_eq!(
        reader
            .read_snapshot(&requested)
            .unwrap()
            .query_transaction_history(None, 10)
            .unwrap()
            .transactions
            .iter()
            .filter(|tx| tx.confirmations == 0)
            .count(),
        2
    );
    fixture
        .database
        .publish_mempool_snapshot(&base, 4, &[])
        .unwrap();
    let snapshot = reader.read_snapshot(&requested).unwrap();
    assert!(
        snapshot
            .query_utxos(None, shared::MAX_UTXO_PAGE_SIZE)
            .unwrap()
            .utxos
            .iter()
            .any(|output| output.transaction_id == hex::encode(hash(1001)))
    );
    assert!(matches!(
        snapshot.query_transaction_history(Some(&cursor), 10),
        Err(Error::CursorChanged)
    ));
}

#[test]
fn stale_missing_and_invalidated_snapshots_do_not_look_empty() {
    let mut fixture = Fixture::new(2);
    let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
    assert!(!reader.query_status().unwrap().ready);
    assert!(matches!(
        reader.read_snapshot(&[]),
        Err(Error::Unavailable(_))
    ));
    ready(&mut fixture, 1);
    assert!(reader.query_status().unwrap().ready);
    let old = reader.read_snapshot(&scripts()).unwrap();
    fixture.database.apply_block(&block(3, 0, vec![])).unwrap();
    assert_eq!(old.status.indexed_height, Some(2));
    assert_eq!(
        old.query_utxos(None, shared::MAX_UTXO_PAGE_SIZE)
            .unwrap()
            .utxos
            .len(),
        2
    );
    drop(old);
    assert!(!reader.query_status().unwrap().ready);
    ready(&mut fixture, 2);
    let writer = Connection::open(fixture.path()).unwrap();
    writer
        .execute("UPDATE sync_status SET last_node_contact = 0", [])
        .unwrap();
    assert!(!reader.query_status().unwrap().ready);
    assert!(matches!(
        reader.read_snapshot(&scripts()),
        Err(Error::Unavailable(_))
    ));
}

#[test]
fn mixed_input_ownership_keeps_the_whole_network_fee_separate_from_net_amount() {
    let mut fixture = Fixture::new(101);
    let mut mixed = payment(2000, 1001, "1023.99999837");
    mixed["vin"]
        .as_array_mut()
        .unwrap()
        .push(json!({"txid": hex::encode(hash(1002)), "vout": 0}));
    let mut block = block(102, 0, vec![mixed]);
    block.transactions[1].outputs[0].script = vec![0x53];
    fixture.database.apply_block(&block).unwrap();
    let writer = Connection::open(fixture.path()).unwrap();
    writer
        .execute(
            "UPDATE outputs SET script = X'52' WHERE txid = ?1",
            [hash(1002).as_slice()],
        )
        .unwrap();
    ready(&mut fixture, 1);
    let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
    let snapshot = reader.read_snapshot(&scripts()[..1]).unwrap();
    let history = snapshot.query_transaction_history(None, 10).unwrap();
    let mixed = history
        .transactions
        .iter()
        .find(|tx| tx.transaction_id == hex::encode(hash(2000)))
        .unwrap();
    assert_eq!(mixed.network_fee, Some(163));
    assert_eq!(mixed.net_amount, -51_200_000_000);
}
