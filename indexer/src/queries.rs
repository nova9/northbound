use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};
use shared::IndexStatus;

use crate::address::ScriptAddress;
use crate::database::{APPLICATION_ID, SCHEMA_VERSION};
use crate::status::read_status;
use crate::{Error, Result};

const MAX_SCRIPTS_PER_SNAPSHOT: usize = 100;

/// Read-only SQLite handle used by the API server to query indexed wallet data.
pub struct IndexReader {
    connection: Connection,
}

/// A single SQLite read transaction holding a consistent view of confirmed blocks,
/// pending mempool rows, and index status metadata.
pub struct ReadSnapshot<'a> {
    pub(crate) database: Transaction<'a>,
    pub status: IndexStatus,
}

impl IndexReader {
    pub fn open(path: &Path, network: &str) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::Unavailable("index database is not available yet".into()))?;
        connection.busy_timeout(Duration::from_secs(3))?;
        connection.pragma_update(None, "foreign_keys", true)?;

        let application_id: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if application_id != APPLICATION_ID || version != SCHEMA_VERSION {
            return Err(Error::Unavailable(
                "index schema is not ready; start the indexer to migrate it".into(),
            ));
        }

        let saved_network: String =
            connection.query_row("SELECT network FROM index_state", [], |row| row.get(0))?;
        if saved_network != network {
            return Err(Error::InvalidData(
                "index network differs from server network".into(),
            ));
        }

        // Store the caller's requested scripts in an in-memory temp table so
        // queries can join against it using the script index without building
        // dynamic SQL `IN (...)` lists.
        connection.execute_batch(
            "PRAGMA temp_store = MEMORY;
             CREATE TEMP TABLE requested_scripts (script BLOB PRIMARY KEY, address TEXT NOT NULL);",
        )?;

        Ok(Self { connection })
    }

    pub fn query_status(&mut self) -> Result<IndexStatus> {
        let transaction = self.connection.transaction()?;
        read_status(&transaction)
    }

    /// Starts a read transaction for the given scripts and checks that the index
    /// is currently ready. Every query on the returned snapshot sees the exact same
    /// database state.
    pub fn read_snapshot(&mut self, scripts: &[ScriptAddress]) -> Result<ReadSnapshot<'_>> {
        if scripts.len() > MAX_SCRIPTS_PER_SNAPSHOT {
            return Err(Error::InvalidRequest(
                "at most 100 scripts are accepted".into(),
            ));
        }

        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM requested_scripts", [])?;

        for entry in scripts {
            transaction.execute(
                "INSERT OR IGNORE INTO requested_scripts VALUES (?1, ?2)",
                params![entry.script, entry.address],
            )?;
        }

        let status = read_status(&transaction)?;
        if !status.ready {
            return Err(Error::Unavailable(
                "index is synchronizing or stale; retry shortly".into(),
            ));
        }

        Ok(ReadSnapshot {
            database: transaction,
            status,
        })
    }
}

impl ReadSnapshot<'_> {
    /// Returns every requested address that has ever received a confirmed or pending output.
    pub fn query_script_activity(&self) -> Result<Vec<String>> {
        let mut statement = self.database.prepare(
            "SELECT address
             FROM requested_scripts r
             WHERE EXISTS (SELECT 1 FROM outputs WHERE script = r.script)
                OR EXISTS (SELECT 1 FROM mempool_outputs WHERE script = r.script)
             ORDER BY address",
        )?;

        let mut addresses = Vec::new();
        for row in statement.query_map([], |row| row.get(0))? {
            addresses.push(row?);
        }
        Ok(addresses)
    }

    pub fn query_transaction_status(
        &self,
        txid: &[u8; 32],
    ) -> Result<shared::TransactionStatusResponse> {
        let confirmed_height = self
            .database
            .query_row(
                "SELECT b.height
                 FROM transactions t
                 JOIN blocks b ON b.hash = t.block_hash
                 WHERE t.txid = ?1",
                [txid.as_slice()],
                |row| read_u64(row, 0),
            )
            .optional()?;

        let is_pending: bool = self.database.query_row(
            "SELECT EXISTS (SELECT 1 FROM mempool_transactions WHERE txid = ?1)",
            [txid.as_slice()],
            |row| row.get(0),
        )?;

        Ok(shared::TransactionStatusResponse {
            known: confirmed_height.is_some() || is_pending,
            block_height: confirmed_height,
        })
    }

    pub(crate) fn snapshot_identity(&self) -> Result<(u64, &str, u64)> {
        let height = self
            .status
            .indexed_height
            .ok_or_else(|| Error::Unavailable("indexed tip is missing".into()))?;
        let tip_hash = self
            .status
            .indexed_hash
            .as_deref()
            .ok_or_else(|| Error::Unavailable("indexed tip hash is missing".into()))?;
        let pending_revision = self
            .status
            .pending_revision
            .ok_or_else(|| Error::Unavailable("pending snapshot revision is missing".into()))?;

        Ok((height, tip_hash, pending_revision))
    }
}

// --- Shared row, confirmation, and cursor helpers ---

pub(crate) fn read_u64(row: &Row<'_>, column: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

pub(crate) fn read_optional_u64(row: &Row<'_>, column: usize) -> rusqlite::Result<Option<u64>> {
    let value: Option<i64> = row.get(column)?;
    match value {
        Some(value) => {
            let unsigned = u64::try_from(value)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))?;
            Ok(Some(unsigned))
        }
        None => Ok(None),
    }
}

pub(crate) fn calculate_confirmations(
    tip_height: u64,
    block_height: u64,
    label: &str,
) -> Result<u64> {
    tip_height
        .checked_sub(block_height)
        .and_then(|difference| difference.checked_add(1))
        .ok_or_else(|| Error::InvalidData(format!("{label} height exceeds indexed tip")))
}

pub(crate) fn encode_cursor<T: Serialize>(cursor: &T, label: &str) -> Result<String> {
    let bytes = serde_json::to_vec(cursor)
        .map_err(|_| Error::InvalidData(format!("cannot encode {label} cursor")))?;
    Ok(hex::encode(bytes))
}

pub(crate) fn decode_cursor<T: DeserializeOwned>(
    text: Option<&str>,
    max_length: usize,
    label: &str,
) -> Result<Option<T>> {
    let Some(text) = text else {
        return Ok(None);
    };

    if text.len() > max_length {
        return Err(Error::InvalidRequest(format!("{label} cursor is too long")));
    }

    let bytes =
        hex::decode(text).map_err(|_| Error::InvalidRequest(format!("invalid {label} cursor")))?;
    let cursor = serde_json::from_slice(&bytes)
        .map_err(|_| Error::InvalidRequest(format!("invalid {label} cursor")))?;
    Ok(Some(cursor))
}

#[cfg(test)]
#[path = "queries_tests.rs"]
mod tests;
