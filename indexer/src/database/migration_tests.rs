use crate::{IndexDatabase, IndexReader};
use rusqlite::{Connection, params};

#[test]
fn migrates_version_one_without_rescanning_or_losing_confirmed_rows() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v1.sqlite");
    let genesis = [1_u8; 32];
    let txid = [2_u8; 32];
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../../migrations/001_index.sql"))
        .unwrap();
    connection
        .execute(
            "INSERT INTO index_state VALUES (1, 'regtest', ?1)",
            [genesis.as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO blocks VALUES (?1, 0, NULL, 1000)",
            [genesis.as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO transactions VALUES (?1, ?2, 0, 1)",
            params![txid.as_slice(), genesis.as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO outputs VALUES (?1, 0, 51200000000, X'6a', 0)",
            [txid.as_slice()],
        )
        .unwrap();
    drop(connection);
    let mut database = IndexDatabase::open(&path, "regtest", genesis).unwrap();
    let tip = database.tip().unwrap().unwrap();
    assert_eq!(tip.height, 0);
    assert_eq!(tip.hash, genesis);
    let mut reader = IndexReader::open(&path, "regtest").unwrap();
    assert!(!reader.query_status().unwrap().ready);
    database.publish_mempool_snapshot(&tip, 1, &[]).unwrap();
    assert!(reader.query_status().unwrap().ready);
    let snapshot = reader.read_snapshot(&[]).unwrap();
    assert!(snapshot.query_transaction_known(&txid).unwrap());
    assert!(snapshot.query_utxos().unwrap().is_empty());
}
