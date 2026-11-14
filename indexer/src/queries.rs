use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Transaction, params};
use shared::{IndexStatus, WalletUtxo};

use crate::address::ScriptAddress;
use crate::database::{APPLICATION_ID, SCHEMA_VERSION};
use crate::{Error, Result};

#[cfg(test)]
mod tests;

pub struct IndexReader {
    connection: Connection,
    network: String,
}

pub struct ReadSnapshot<'a> {
    pub(crate) database: Transaction<'a>,
    pub status: IndexStatus,
    pub(crate) network: &'a str,
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
        connection.execute_batch(
            "PRAGMA temp_store = MEMORY;
            CREATE TEMP TABLE requested_scripts (script BLOB PRIMARY KEY, address TEXT NOT NULL);",
        )?;
        Ok(Self {
            connection,
            network: network.into(),
        })
    }

    pub fn query_status(&mut self) -> Result<IndexStatus> {
        let transaction = self.connection.transaction()?;
        crate::status::read_status(&transaction)
    }

    /// Hold a SQLite read transaction for metadata and every query in this response.
    pub fn read_snapshot(&mut self, scripts: &[ScriptAddress]) -> Result<ReadSnapshot<'_>> {
        if scripts.len() > 100 {
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
        let status = crate::status::read_status(&transaction)?;
        if !status.ready {
            return Err(Error::Unavailable(
                "index is synchronizing or stale; retry shortly".into(),
            ));
        }
        Ok(ReadSnapshot {
            database: transaction,
            status,
            network: &self.network,
        })
    }
}

impl ReadSnapshot<'_> {
    pub fn query_utxos(&self) -> Result<Vec<WalletUtxo>> {
        let height = self
            .status
            .indexed_height
            .ok_or_else(|| Error::Unavailable("indexed tip is missing".into()))?;
        // Keep the small requested set first so SQLite seeks by script instead
        // of scanning every output and checking whether it was requested.
        let mut statement = self.database.prepare(
            "SELECT o.txid, o.vout, o.amount, o.script, b.height, t.is_coinbase
             FROM requested_scripts r CROSS JOIN outputs o ON o.script = r.script
             JOIN transactions t ON t.txid = o.txid JOIN blocks b ON b.hash = t.block_hash
             WHERE o.utxo_eligible = 1
               AND NOT EXISTS (SELECT 1 FROM spends WHERE previous_txid = o.txid AND previous_vout = o.vout)
               AND NOT EXISTS (SELECT 1 FROM mempool_spends WHERE previous_txid = o.txid AND previous_vout = o.vout)
             ORDER BY b.height, o.txid, o.vout LIMIT 10001",
        )?;
        let rows = statement.query_map([], |row| {
            let txid: Vec<u8> = row.get(0)?;
            let script: Vec<u8> = row.get(3)?;
            let block_height = read_u64(row, 4)?;
            Ok(WalletUtxo {
                transaction_id: hex::encode(txid),
                output_index: row.get(1)?,
                amount: read_u64(row, 2)?,
                script_pubkey: hex::encode(script),
                block_height,
                confirmations: height - block_height + 1,
                coinbase: row.get(5)?,
            })
        })?;
        let mut outputs = Vec::new();
        for row in rows {
            outputs.push(row?);
        }
        if outputs.len() > 10_000 {
            return Err(Error::InvalidRequest(
                "request exceeds the 10,000 output limit".into(),
            ));
        }
        Ok(outputs)
    }

    pub fn query_script_activity(&self) -> Result<Vec<String>> {
        let mut statement = self.database.prepare(
            "SELECT address FROM requested_scripts r
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

    pub fn query_transaction_known(&self, txid: &[u8; 32]) -> Result<bool> {
        Ok(self.database.query_row(
            "SELECT EXISTS (SELECT 1 FROM transactions WHERE txid = ?1)
                OR EXISTS (SELECT 1 FROM mempool_transactions WHERE txid = ?1)",
            [txid.as_slice()],
            |row| row.get(0),
        )?)
    }
}

pub(crate) fn read_u64(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

pub(crate) fn read_optional_u64(
    row: &rusqlite::Row<'_>,
    column: usize,
) -> rusqlite::Result<Option<u64>> {
    let value: Option<i64> = row.get(column)?;
    value
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
        })
        .transpose()
}
