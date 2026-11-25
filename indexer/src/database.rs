use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use truenorth_rpc::{MAX_MONEY, NodeTip, Outpoint, Transaction as ChainTransaction, decode_hash};

use crate::status::unix_time;
use crate::{Block, Error, Hash, Result};

pub(crate) const APPLICATION_ID: i64 = 0x4e425849;
pub(crate) const SCHEMA_VERSION: i64 = 2;

// TrueNorth v30.5.4 consensus rules used when validating spendable outputs.
const COINBASE_MATURITY: i64 = 100;
const OP_RETURN: u8 = 0x6a;
const MAX_SPENDABLE_SCRIPT_BYTES: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainTip {
    pub height: i64,
    pub hash: Hash,
}

/// Exclusive writer handle for the SQLite index database.
pub struct IndexDatabase {
    connection: Connection,
    // Keep the OS file lock open for the writer's lifetime. Never unlink a live lock file.
    _writer_lock: File,
    genesis_hash: Hash,
}

struct PreviousOutput {
    amount: i64,
    utxo_eligible: bool,
    is_coinbase: bool,
    block_height: Option<i64>,
    is_spent: bool,
}

impl IndexDatabase {
    pub fn open(path: &Path, network: &str, genesis_hash: Hash) -> Result<Self> {
        if network != "main" && network != "regtest" {
            return Err(Error::InvalidData("network must be main or regtest".into()));
        }

        let canonical_path = canonicalize_database_path(path)?;
        let writer_lock = acquire_writer_lock(&canonical_path)?;

        let mut connection = Connection::open(canonical_path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;

        initialize_schema(&mut connection, network, &genesis_hash)?;

        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        invalidate_pending_snapshot(&connection)?;

        Ok(Self {
            connection,
            _writer_lock: writer_lock,
            genesis_hash,
        })
    }

    pub fn tip(&self) -> Result<Option<ChainTip>> {
        read_tip(&self.connection)
    }

    pub fn mark_unavailable(&self) -> Result<()> {
        self.connection
            .execute("UPDATE sync_status SET ready = 0", [])?;
        Ok(())
    }

    pub(crate) fn record_node_observation(&self, tip: &NodeTip) -> Result<()> {
        let tip_hash = decode_hash(&tip.bestblockhash)?;
        self.connection.execute(
            "UPDATE sync_status
             SET observed_height = ?1, observed_hash = ?2, last_node_contact = ?3",
            params![tip.blocks, tip_hash.as_slice(), unix_time()?],
        )?;
        Ok(())
    }

    pub(crate) fn saved_mempool_state(&self) -> Result<(Option<Vec<u8>>, Option<i64>)> {
        let row = self.connection.query_row(
            "SELECT pending_base_hash, pending_sequence FROM sync_status",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(row)
    }

    pub(crate) fn refresh_mempool_timestamp(&self) -> Result<()> {
        self.connection.execute(
            "UPDATE sync_status
             SET ready = 1, pending_observed_at = ?1, last_node_contact = ?1",
            [unix_time()?],
        )?;
        Ok(())
    }

    // --- Confirmed blocks ---

    /// Validates and writes one confirmed block atomically.
    /// Any error rolls back the SQLite transaction, leaving the previous tip intact.
    pub fn apply_block(&mut self, block: &Block) -> Result<()> {
        block.validate()?;

        let db = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        verify_block_extends_tip(&db, block, &self.genesis_hash)?;
        insert_confirmed_block(&db, block)?;
        invalidate_pending_snapshot(&db)?;

        db.commit()?;
        Ok(())
    }

    /// Removes the current tip block during a chain reorganization.
    /// Foreign keys with `ON DELETE CASCADE` clean up owned transactions, outputs, and spends.
    pub fn rollback_tip(&mut self, expected: &ChainTip) -> Result<()> {
        let db = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if read_tip(&db)?.as_ref() != Some(expected) {
            return Err(Error::InvalidData(
                "indexed tip changed before rollback".into(),
            ));
        }

        if expected.height == 0 {
            return Err(Error::InvalidData(
                "cannot roll back genesis; verify the node identity".into(),
            ));
        }

        // Foreign keys on previous outpoints use NO ACTION, so SQLite refuses to
        // delete an output that is still referenced by a surviving spend.
        db.execute(
            "DELETE FROM blocks WHERE hash = ?1",
            [expected.hash.as_slice()],
        )?;
        invalidate_pending_snapshot(&db)?;

        db.commit()?;
        Ok(())
    }

    // --- Pending mempool snapshots ---

    /// Atomically replaces the pending mempool tables with a coherent snapshot.
    /// Callers must pass transactions in parent-before-child order so child spends
    /// can resolve outputs created by their unconfirmed parents.
    pub fn publish_mempool_snapshot(
        &mut self,
        base: &ChainTip,
        sequence: u64,
        transactions: &[ChainTransaction],
    ) -> Result<()> {
        let observed_at = unix_time()?;
        let spending_height = base
            .height
            .checked_add(1)
            .ok_or_else(|| Error::InvalidData("pending spending height overflow".into()))?;
        let sequence = i64::try_from(sequence)
            .map_err(|_| Error::InvalidData("mempool sequence overflow".into()))?;

        let db = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if read_tip(&db)?.as_ref() != Some(base) {
            return Err(Error::Unavailable("snapshot base tip changed".into()));
        }

        // Preserve first-seen timestamps for transactions that stayed in the mempool.
        let previous_first_seen = load_mempool_first_seen(&db)?;
        db.execute("DELETE FROM mempool_transactions", [])?;

        for transaction in transactions {
            let first_seen_at = previous_first_seen
                .get(transaction.txid.as_slice())
                .copied()
                .unwrap_or(observed_at);
            insert_pending_transaction(&db, transaction, spending_height, first_seen_at)?;
        }

        db.execute(
            "UPDATE sync_status
             SET ready = 1,
                 observed_height = ?1,
                 observed_hash = ?2,
                 last_node_contact = ?3,
                 pending_base_hash = ?2,
                 pending_sequence = ?4,
                 pending_observed_at = ?3,
                 pending_revision = pending_revision + 1",
            params![base.height, base.hash.as_slice(), observed_at, sequence],
        )?;

        db.commit()?;
        Ok(())
    }
}

// --- Opening, locking, and migrations ---

fn canonicalize_database_path(path: &Path) -> Result<PathBuf> {
    // Canonicalize the parent directory so relative paths and symlinks share one writer lock.
    if path.exists() {
        return Ok(path.canonicalize()?);
    }

    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let file_name = path
        .file_name()
        .ok_or_else(|| Error::InvalidData("database path must name a file".into()))?;

    Ok(parent.canonicalize()?.join(file_name))
}

fn acquire_writer_lock(database_path: &Path) -> Result<File> {
    let mut lock_path = database_path.as_os_str().to_os_string();
    lock_path.push(".writer.lock");

    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;

    lock_file.try_lock().map_err(|error| {
        Error::InvalidData(format!(
            "cannot acquire index writer lock; another indexer may be running: {error}"
        ))
    })?;

    Ok(lock_file)
}

fn initialize_schema(connection: &mut Connection, network: &str, genesis: &Hash) -> Result<()> {
    let db = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let application_id: i64 = db.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;

    if application_id == 0 && version == 0 {
        let existing_tables: i64 = db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if existing_tables != 0 {
            return Err(Error::InvalidData(
                "unrecognized existing database; use a new file, leaving the manual database intact"
                    .into(),
            ));
        }

        db.execute_batch(include_str!("../migrations/001_index.sql"))?;
        db.execute(
            "INSERT INTO index_state (singleton, network, genesis_hash) VALUES (1, ?1, ?2)",
            params![network, genesis.as_slice()],
        )?;
    } else if application_id != APPLICATION_ID || !(1..=SCHEMA_VERSION).contains(&version) {
        return Err(Error::InvalidData(
            "unsupported index database schema".into(),
        ));
    }

    let (saved_network, saved_genesis): (String, Vec<u8>) = db.query_row(
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
        db.execute_batch(include_str!("../migrations/002_pending.sql"))?;
    }

    db.commit()?;
    Ok(())
}

fn invalidate_pending_snapshot(connection: &Connection) -> Result<()> {
    connection.execute(
        "UPDATE sync_status SET ready = 0, pending_base_hash = NULL",
        [],
    )?;
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

    let Some((height, hash_bytes)) = row else {
        return Ok(None);
    };

    let hash = hash_bytes
        .try_into()
        .map_err(|_| Error::InvalidData("stored block hash has an invalid length".into()))?;
    Ok(Some(ChainTip { height, hash }))
}

// --- Block and transaction writing helpers ---

fn verify_block_extends_tip(
    db: &Transaction<'_>,
    block: &Block,
    genesis_hash: &Hash,
) -> Result<()> {
    match read_tip(db)? {
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
            if block.height != 0 || block.hash != *genesis_hash {
                return Err(Error::InvalidData(
                    "first block must match the configured genesis".into(),
                ));
            }
        }
    }

    Ok(())
}

fn insert_confirmed_block(db: &Transaction<'_>, block: &Block) -> Result<()> {
    db.execute(
        "INSERT INTO blocks (hash, height, previous_hash, timestamp) VALUES (?1, ?2, ?3, ?4)",
        params![
            block.hash.as_slice(),
            block.height,
            block.previous_hash.as_ref().map(|hash| hash.as_slice()),
            block.timestamp
        ],
    )?;

    for (position, transaction) in block.transactions.iter().enumerate() {
        db.execute(
            "INSERT INTO transactions (txid, block_hash, position, is_coinbase)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                transaction.txid.as_slice(),
                block.hash.as_slice(),
                position as i64,
                transaction.is_coinbase
            ],
        )?;

        // Resolve inputs before inserting this transaction's outputs so a
        // transaction can never spend one of its own outputs.
        let mut input_total = 0_i64;
        for (vin, input) in transaction.inputs.iter().enumerate() {
            let previous = lookup_confirmed_output(db, input)?;
            let amount = validate_previous_output(previous, input, block.height)?;
            input_total = checked_add_money(input_total, amount, "transaction input total")?;

            db.execute(
                "INSERT INTO spends (spending_txid, vin, previous_txid, previous_vout)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    transaction.txid.as_slice(),
                    vin as i64,
                    input.txid.as_slice(),
                    input.vout
                ],
            )?;
        }

        let mut output_total = 0_i64;
        for (vout, output) in transaction.outputs.iter().enumerate() {
            output_total = output_total
                .checked_add(output.amount)
                .ok_or_else(|| Error::InvalidData("output total overflow".into()))?;

            // TrueNorth Core skips connecting genesis outputs; they remain history-only.
            let eligible = block.height != 0 && is_spendable_script(&output.script);
            db.execute(
                "INSERT INTO outputs (txid, vout, amount, script, utxo_eligible)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    transaction.txid.as_slice(),
                    vout as i64,
                    output.amount,
                    output.script,
                    eligible
                ],
            )?;
        }

        if !transaction.is_coinbase && input_total < output_total {
            return Err(Error::InvalidData(
                "transaction outputs exceed its inputs".into(),
            ));
        }
    }

    Ok(())
}

fn load_mempool_first_seen(db: &Transaction<'_>) -> Result<BTreeMap<Vec<u8>, i64>> {
    let mut statement = db.prepare("SELECT txid, first_seen_at FROM mempool_transactions")?;
    let mut first_seen = BTreeMap::new();

    for row in statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))? {
        let (txid, timestamp) = row?;
        first_seen.insert(txid, timestamp);
    }

    Ok(first_seen)
}

fn insert_pending_transaction(
    db: &Transaction<'_>,
    transaction: &ChainTransaction,
    spending_height: i64,
    first_seen_at: i64,
) -> Result<()> {
    if transaction.is_coinbase {
        return Err(Error::InvalidData("pending coinbase transaction".into()));
    }

    let already_confirmed: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM transactions WHERE txid = ?1)",
        [transaction.txid.as_slice()],
        |row| row.get(0),
    )?;
    if already_confirmed {
        return Err(Error::InvalidData(
            "pending transaction is already confirmed".into(),
        ));
    }

    db.execute(
        "INSERT INTO mempool_transactions VALUES (?1, ?2)",
        params![transaction.txid.as_slice(), first_seen_at],
    )?;

    let mut input_total = 0_i64;
    for (vin, input) in transaction.inputs.iter().enumerate() {
        let previous = lookup_any_output(db, input)?;
        let amount = validate_previous_output(previous, input, spending_height)?;
        input_total = checked_add_money(input_total, amount, "pending input total")?;

        db.execute(
            "INSERT INTO mempool_spends VALUES (?1, ?2, ?3, ?4)",
            params![
                transaction.txid.as_slice(),
                vin as i64,
                input.txid.as_slice(),
                input.vout
            ],
        )?;
    }

    let mut output_total = 0_i64;
    for (vout, output) in transaction.outputs.iter().enumerate() {
        if output.amount < 0 {
            return Err(Error::InvalidData("negative pending output".into()));
        }
        output_total = checked_add_money(output_total, output.amount, "pending output total")?;

        db.execute(
            "INSERT INTO mempool_outputs VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                transaction.txid.as_slice(),
                vout as i64,
                output.amount,
                output.script,
                is_spendable_script(&output.script)
            ],
        )?;
    }

    if transaction.inputs.is_empty() || transaction.outputs.is_empty() || input_total < output_total
    {
        return Err(Error::InvalidData(
            "invalid pending transaction totals or counts".into(),
        ));
    }

    Ok(())
}

fn lookup_confirmed_output(
    db: &Transaction<'_>,
    input: &Outpoint,
) -> Result<Option<PreviousOutput>> {
    let output = db
        .query_row(
            "SELECT o.amount,
                    o.utxo_eligible,
                    t.is_coinbase,
                    b.height,
                    EXISTS (
                        SELECT 1 FROM spends
                        WHERE previous_txid = o.txid AND previous_vout = o.vout
                    )
             FROM outputs o
             JOIN transactions t ON t.txid = o.txid
             JOIN blocks b ON b.hash = t.block_hash
             WHERE o.txid = ?1 AND o.vout = ?2",
            params![input.txid.as_slice(), input.vout],
            |row| {
                Ok(PreviousOutput {
                    amount: row.get(0)?,
                    utxo_eligible: row.get(1)?,
                    is_coinbase: row.get(2)?,
                    block_height: Some(row.get(3)?),
                    is_spent: row.get(4)?,
                })
            },
        )
        .optional()?;

    Ok(output)
}

fn lookup_any_output(db: &Transaction<'_>, input: &Outpoint) -> Result<Option<PreviousOutput>> {
    let output = db
        .query_row(
            "SELECT o.amount,
                    o.utxo_eligible,
                    coalesce(t.is_coinbase, 0),
                    b.height,
                    EXISTS (
                        SELECT 1 FROM all_spends
                        WHERE previous_txid = o.txid AND previous_vout = o.vout
                    )
             FROM all_outputs o
             LEFT JOIN transactions t ON t.txid = o.txid
             LEFT JOIN blocks b ON b.hash = t.block_hash
             WHERE o.txid = ?1 AND o.vout = ?2",
            params![input.txid.as_slice(), input.vout],
            |row| {
                Ok(PreviousOutput {
                    amount: row.get(0)?,
                    utxo_eligible: row.get(1)?,
                    is_coinbase: row.get(2)?,
                    block_height: row.get(3)?,
                    is_spent: row.get(4)?,
                })
            },
        )
        .optional()?;

    Ok(output)
}

fn validate_previous_output(
    previous: Option<PreviousOutput>,
    input: &Outpoint,
    spending_height: i64,
) -> Result<i64> {
    let outpoint = format!("{}:{}", hex::encode(input.txid), input.vout);
    let Some(output) = previous else {
        return Err(Error::InvalidData(format!("missing input {outpoint}")));
    };

    if !output.utxo_eligible || output.is_spent {
        return Err(Error::InvalidData(format!(
            "input {outpoint} is ineligible or already spent"
        )));
    }

    if output.is_coinbase {
        let is_mature = output
            .block_height
            .is_some_and(|created| spending_height - created >= COINBASE_MATURITY);
        if !is_mature {
            return Err(Error::InvalidData(format!(
                "input {outpoint} is an immature coinbase"
            )));
        }
    }

    Ok(output.amount)
}

/// Matches `CScript::IsUnspendable` in TrueNorth Core: excludes `OP_RETURN`
/// and scripts larger than 10,000 bytes.
fn is_spendable_script(script: &[u8]) -> bool {
    script.first() != Some(&OP_RETURN) && script.len() <= MAX_SPENDABLE_SCRIPT_BYTES
}

fn checked_add_money(total: i64, amount: i64, label: &str) -> Result<i64> {
    let sum = total
        .checked_add(amount)
        .ok_or_else(|| Error::InvalidData(format!("{label} overflow")))?;
    if sum > MAX_MONEY {
        return Err(Error::InvalidData(format!("{label} exceeds MAX_MONEY")));
    }
    Ok(sum)
}

#[cfg(test)]
#[path = "database_tests.rs"]
mod tests;
