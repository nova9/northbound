use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{Block, Error, Hash, Result};

#[cfg(test)]
mod crash_tests;
#[cfg(test)]
mod migration_tests;
#[cfg(test)]
mod tests;

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
