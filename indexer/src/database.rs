use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{Block, Error, Hash, Result};

pub(crate) const APPLICATION_ID: i64 = 0x4e425849;
pub(crate) const SCHEMA_VERSION: i64 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainTip {
    pub height: i64,
    pub hash: Hash,
}

pub struct IndexDatabase {
    pub(crate) connection: Connection,
    // Retain the OS lock for the writer's lifetime. Never unlink a live lock file.
    _writer_lock: File,
    genesis_hash: Hash,
}

impl IndexDatabase {
    pub fn open(path: &Path, network: &str, genesis_hash: Hash) -> Result<Self> {
        if network != "main" && network != "regtest" {
            return Err(Error::InvalidData("network must be main or regtest".into()));
        }
        // Canonicalize the parent so relative paths and symlinked directories
        // share a lock. Existing database symlinks resolve to their target.
        let path = if path.exists() {
            path.canonicalize()?
        } else {
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let name = path
                .file_name()
                .ok_or_else(|| Error::InvalidData("database path must name a file".into()))?;
            parent.canonicalize()?.join(name)
        };
        let mut lock_name = path.as_os_str().to_os_string();
        lock_name.push(".writer.lock");
        let writer_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_name)?;
        writer_lock.try_lock().map_err(|error| {
            Error::InvalidData(format!(
                "cannot acquire index writer lock; another indexer may be running: {error}"
            ))
        })?;

        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        initialize_schema(&mut connection, network, &genesis_hash)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.execute(
            "UPDATE sync_status SET ready = 0, pending_base_hash = NULL",
            [],
        )?;
        Ok(Self {
            connection,
            _writer_lock: writer_lock,
            genesis_hash,
        })
    }

    pub fn tip(&self) -> Result<Option<ChainTip>> {
        read_tip(&self.connection)
    }

    pub fn apply_block(&mut self, block: &Block) -> Result<()> {
        block.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let tip = read_tip(&transaction)?;
        match tip {
            Some(tip) => {
                if tip.height.checked_add(1) != Some(block.height)
                    || block.previous_hash != Some(tip.hash)
                {
                    return Err(Error::InvalidData(
                        "block does not extend the indexed tip".into(),
                    ));
                }
            }
            None => {
                if block.height != 0 || block.hash != self.genesis_hash {
                    return Err(Error::InvalidData(
                        "first block must match the configured genesis".into(),
                    ));
                }
            }
        }
        crate::store_block::insert_block_records(&transaction, block)?;
        transaction.execute(
            "UPDATE sync_status SET ready = 0, pending_base_hash = NULL",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn rollback_tip(&mut self, expected: &ChainTip) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read_tip(&transaction)?.as_ref() != Some(expected) {
            return Err(Error::InvalidData(
                "indexed tip changed before rollback".into(),
            ));
        }
        if expected.height == 0 {
            return Err(Error::InvalidData(
                "cannot roll back genesis; verify the node identity".into(),
            ));
        }
        // Cascades remove owned rows. NO ACTION on previous outpoints prevents
        // deleting outputs still referenced by surviving transactions.
        transaction.execute(
            "DELETE FROM blocks WHERE hash = ?1",
            [expected.hash.as_slice()],
        )?;
        transaction.execute(
            "UPDATE sync_status SET ready = 0, pending_base_hash = NULL",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn initialize_schema(connection: &mut Connection, network: &str, genesis: &Hash) -> Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let application_id: i64 =
        transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application_id == 0 && version == 0 {
        let tables: i64 = transaction.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if tables != 0 {
            return Err(Error::InvalidData("unrecognized existing database; use a new file, leaving the manual database intact".into()));
        }
        transaction.execute_batch(include_str!("../migrations/001_index.sql"))?;
        transaction.execute(
            "INSERT INTO index_state (singleton, network, genesis_hash) VALUES (1, ?1, ?2)",
            params![network, genesis.as_slice()],
        )?;
    } else if application_id != APPLICATION_ID || !(1..=SCHEMA_VERSION).contains(&version) {
        return Err(Error::InvalidData(
            "unsupported index database schema".into(),
        ));
    }
    let (saved_network, saved_genesis): (String, Vec<u8>) = transaction.query_row(
        "SELECT network, genesis_hash FROM index_state WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if saved_network != network || saved_genesis != genesis {
        return Err(Error::InvalidData(
            "database network or genesis does not match the node".into(),
        ));
    }
    if version < 2 {
        transaction.execute_batch(include_str!("../migrations/002_pending.sql"))?;
    }
    transaction.commit()?;
    Ok(())
}

pub(crate) fn read_tip(connection: &Connection) -> Result<Option<ChainTip>> {
    let row: Option<(i64, Vec<u8>)> = connection
        .query_row(
            "SELECT height, hash FROM blocks ORDER BY height DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((height, hash)) => {
            let hash = hash.try_into().map_err(|_| {
                Error::InvalidData("stored block hash has an invalid length".into())
            })?;
            Ok(Some(ChainTip { height, hash }))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::{env, path::PathBuf, process::Command};

    use rusqlite::{Connection, params};
    use serde_json::json;

    use crate::test_support::{Fixture, block, coinbase, hash, payment};
    use crate::{IndexDatabase, IndexReader, decode_block, decode_hash};

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
    fn migrates_version_one_without_rescanning_or_losing_confirmed_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("v1.sqlite");
        let genesis = [1_u8; 32];
        let txid = [2_u8; 32];
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("../migrations/001_index.sql"))
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
        assert!(
            snapshot
                .query_utxos(None, shared::MAX_UTXO_PAGE_SIZE)
                .unwrap()
                .utxos
                .is_empty()
        );
    }

    const GENESIS: [u8; 32] = [1; 32];
    const TRANSACTION_ID: [u8; 32] = [2; 32];

    #[test]
    fn abrupt_exit_before_and_after_commit_preserves_atomic_progress() {
        for mode in ["before", "after"] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("chain.sqlite");
            let output = Command::new(env::current_exe().unwrap())
                .args(["--exact", "database::tests::crash_worker"])
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
}
