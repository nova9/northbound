use crate::test_support::{Fixture, block, coinbase, hash, payment};
use crate::{IndexDatabase, decode_block, decode_hash};
use rusqlite::{Connection, params};
use serde_json::json;

#[test]
fn preserves_genesis_scripts_and_hash_display_order() {
    let mut fixture = Fixture::new(0);
    let genesis: (Vec<u8>, bool) = fixture
        .reader()
        .query_row("SELECT script, utxo_eligible FROM outputs", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(genesis, (vec![0x51], false));
    assert_eq!(fixture.database.tip().unwrap().unwrap().hash, hash(1));
    let hash_text = "d911bd3ec7ba0f597643c3920f4d6e01ffd872fcae38e0aaab13eb42718aed2f";
    assert_eq!(hex::encode(decode_hash(hash_text).unwrap()), hash_text);

    let mut transaction = coinbase(
        1001,
        "5220d57d900d942200b8431ba4480f50edc29ca45e1ab476c5be60e628fe7b2ec551",
    );
    for (index, script) in ["6a", &"51".repeat(10_001), ""].iter().enumerate() {
        transaction["vout"].as_array_mut().unwrap().push(json!({
            "n": index + 1, "value": 0, "scriptPubKey": {"hex": script}
        }));
    }
    let block = decode_block(
        &json!({
            "hash": hex::encode(hash(2)), "height": 1, "previousblockhash": hex::encode(hash(1)),
            "time": 1001, "tx": [transaction]
        })
        .to_string(),
    )
    .unwrap();
    fixture.database.apply_block(&block).unwrap();
    let reader = fixture.reader();
    let mut statement = reader
        .prepare("SELECT utxo_eligible FROM outputs WHERE txid = ?1 ORDER BY vout")
        .unwrap();
    let flags: Vec<bool> = statement
        .query_map([hash(1001).as_slice()], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(flags, vec![true, false, false, true]);
}

#[test]
fn same_block_parent_child_and_cascade_restore_previous_output() {
    let mut fixture = Fixture::new(100);
    let original_tip = fixture.database.tip().unwrap();
    let parent = payment(2000, 1001, "511.99999837");
    let child = payment(2001, 2000, "511.99999674");
    fixture
        .database
        .apply_block(&block(101, 0, vec![parent, child]))
        .unwrap();
    assert_eq!(fixture.count("spends"), 2);
    assert_eq!(fixture.count("transactions"), 104);
    let tip = fixture.database.tip().unwrap().unwrap();
    fixture.database.rollback_tip(&tip).unwrap();
    assert_eq!(fixture.database.tip().unwrap(), original_tip);
    assert_eq!(fixture.count("spends"), 0);
    assert_eq!(fixture.count("outputs"), 101);
    assert_eq!(fixture.count("transactions"), 101);
    let restored: i64 = fixture.reader().query_row(
        "SELECT amount FROM outputs WHERE txid = ?1 AND vout = 0
         AND NOT EXISTS (SELECT 1 FROM spends WHERE previous_txid = outputs.txid AND previous_vout = outputs.vout)",
        [hash(1001).as_slice()], |row| row.get(0),
    ).unwrap();
    assert_eq!(restored, 51_200_000_000);
    fixture
        .database
        .apply_block(&block(101, 1, vec![payment(3000, 1001, "510")]))
        .unwrap();
    assert_eq!(fixture.count("spends"), 1);
    assert_eq!(fixture.database.tip().unwrap().unwrap().hash, hash(10_102));
}

#[test]
fn failed_block_application_leaves_no_partial_records() {
    let mut fixture = Fixture::new(100);
    for transactions in [
        vec![payment(2000, 9999, "1")],   // Missing previous output.
        vec![payment(2000, 1001, "513")], // Outputs exceed inputs.
        vec![payment(2000, 1001, "500"), payment(2001, 1001, "1")], // Double spend.
        vec![payment(2000, 1100, "1")],   // Immature coinbase.
        vec![payment(2000, 1000, "1")],   // Genesis is ineligible.
        vec![payment(1001, 1002, "1")],   // Duplicate transaction ID.
    ] {
        let error = fixture.database.apply_block(&block(101, 0, transactions));
        assert!(error.is_err());
        assert_eq!(fixture.database.tip().unwrap().unwrap().height, 100);
        assert_eq!(fixture.count("blocks"), 101);
        assert_eq!(fixture.count("transactions"), 101);
        assert_eq!(fixture.count("outputs"), 101);
        assert_eq!(fixture.count("spends"), 0);
    }
}

#[test]
fn rejects_wrong_parent_and_preserves_saved_tip_on_restart() {
    let mut fixture = Fixture::new(2);
    assert!(fixture.database.apply_block(&block(4, 0, vec![])).is_err());
    let mut wrong_parent = json!({
        "hash": hex::encode(hash(4)), "height": 3, "previousblockhash": hex::encode(hash(999)),
        "time": 1003, "tx": [coinbase(1003, "51")]
    });
    assert!(
        fixture
            .database
            .apply_block(&decode_block(&wrong_parent.to_string()).unwrap())
            .is_err()
    );
    wrong_parent["previousblockhash"] = json!(hex::encode(hash(3)));
    fixture
        .database
        .apply_block(&decode_block(&wrong_parent.to_string()).unwrap())
        .unwrap();
    let tip = fixture.database.tip().unwrap();
    let path = fixture.path();
    drop(fixture.database);
    let reopened = IndexDatabase::open(&path, "regtest", hash(1)).unwrap();
    assert_eq!(reopened.tip().unwrap(), tip);
}

#[test]
fn refuses_second_writer_wrong_identity_and_manual_database() {
    let fixture = Fixture::new(0);
    let path = fixture.path();
    assert!(IndexDatabase::open(&path, "regtest", hash(1)).is_err());
    drop(fixture.database);
    assert!(IndexDatabase::open(&path, "main", hash(1)).is_err());
    assert!(IndexDatabase::open(&path, "regtest", hash(2)).is_err());
    assert!(IndexDatabase::open(&path, "regtest", hash(1)).is_ok());

    let manual_path = fixture.directory.path().join("manual.sqlite");
    let manual = Connection::open(&manual_path).unwrap();
    manual
        .execute_batch("CREATE TABLE blocks (height INTEGER); INSERT INTO blocks VALUES (42);")
        .unwrap();
    assert!(IndexDatabase::open(&manual_path, "regtest", hash(1)).is_err());
    let saved_height: i64 = manual
        .query_row("SELECT height FROM blocks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(saved_height, 42);
}

#[test]
fn foreign_keys_block_out_of_order_deletion_and_reader_sees_consistent_snapshot() {
    let mut fixture = Fixture::new(100);
    fixture
        .database
        .apply_block(&block(101, 0, vec![payment(2000, 1001, "500")]))
        .unwrap();
    let mut writer = Connection::open(fixture.path()).unwrap();
    writer.pragma_update(None, "foreign_keys", true).unwrap();
    assert!(
        writer
            .execute("DELETE FROM blocks WHERE height = 1", [])
            .is_err()
    );

    let mut reader = fixture.reader();
    let snapshot = reader.transaction().unwrap();
    let old_count: i64 = snapshot
        .query_row("SELECT count(*) FROM blocks", [], |row| row.get(0))
        .unwrap();
    fixture
        .database
        .apply_block(&block(102, 0, vec![]))
        .unwrap();
    let same_count: i64 = snapshot
        .query_row("SELECT count(*) FROM blocks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(old_count, same_count);
    snapshot.commit().unwrap();

    // An uncommitted write is rolled back when its connection is dropped.
    {
        let pending = writer.transaction().unwrap();
        pending.execute("INSERT INTO blocks (hash, height, previous_hash, timestamp) VALUES (?1, 103, ?2, 1)",
            params![hash(104).as_slice(), hash(103).as_slice()]).unwrap();
    }
    drop(writer);
    assert_eq!(fixture.database.tip().unwrap().unwrap().height, 102);
}

#[test]
fn malformed_rpc_block_fields_are_rejected() {
    let valid = json!({
        "hash": hex::encode(hash(1)), "height": 0, "time": 1000, "tx": [coinbase(1000, "6a")]
    });
    for (pointer, value) in [
        ("/hash", json!("not hex")),
        ("/height", json!(-1)),
        ("/tx/0/vout/0/n", json!(2)),
        ("/tx/0/vout/0/value", json!(-1)),
        ("/tx/0/vout/0/scriptPubKey/hex", json!("zz")),
        ("/tx/0/vin", json!([])),
    ] {
        let mut malformed = valid.clone();
        *malformed.pointer_mut(pointer).unwrap() = value;
        assert!(decode_block(&malformed.to_string()).is_err(), "{pointer}");
    }
}
