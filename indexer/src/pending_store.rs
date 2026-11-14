use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use truenorth_rpc::{MAX_MONEY, Transaction};

use crate::database::read_tip;
use crate::status::unix_time;
use crate::{ChainTip, Error, IndexDatabase, Result};

impl IndexDatabase {
    /// Publish transactions in parent-before-child order after rechecking the
    /// node's tip and mempool sequence. Readers see only the complete snapshot.
    pub fn publish_mempool_snapshot(
        &mut self,
        base: &ChainTip,
        sequence: u64,
        transactions: &[Transaction],
    ) -> Result<()> {
        let observed_at = unix_time()?;
        let spending_height = base
            .height
            .checked_add(1)
            .ok_or_else(|| Error::InvalidData("pending spending height overflow".into()))?;
        let sequence = i64::try_from(sequence)
            .map_err(|_| Error::InvalidData("mempool sequence overflow".into()))?;
        let database = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read_tip(&database)?.as_ref() != Some(base) {
            return Err(Error::Unavailable("snapshot base tip changed".into()));
        }
        let mut first_seen = BTreeMap::<Vec<u8>, i64>::new();
        {
            let mut statement =
                database.prepare("SELECT txid, first_seen_at FROM mempool_transactions")?;
            for row in statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))? {
                let (id, time) = row?;
                first_seen.insert(id, time);
            }
        }
        database.execute("DELETE FROM mempool_transactions", [])?;
        for transaction in transactions {
            if transaction.is_coinbase {
                return Err(Error::InvalidData("pending coinbase transaction".into()));
            }
            let confirmed: bool = database.query_row(
                "SELECT EXISTS (SELECT 1 FROM transactions WHERE txid = ?1)",
                [transaction.txid.as_slice()],
                |row| row.get(0),
            )?;
            if confirmed {
                return Err(Error::InvalidData(
                    "pending transaction is already confirmed".into(),
                ));
            }
            database.execute(
                "INSERT INTO mempool_transactions VALUES (?1, ?2)",
                params![
                    transaction.txid.as_slice(),
                    first_seen
                        .get(transaction.txid.as_slice())
                        .copied()
                        .unwrap_or(observed_at)
                ],
            )?;
            let mut input_total = 0_i64;
            for (vin, input) in transaction.inputs.iter().enumerate() {
                let previous: Option<(i64, bool, bool, Option<i64>, bool)> = database.query_row(
                    "SELECT o.amount, o.utxo_eligible, coalesce(t.is_coinbase, 0), b.height,
                        EXISTS (SELECT 1 FROM all_spends WHERE previous_txid = o.txid AND previous_vout = o.vout)
                     FROM all_outputs o LEFT JOIN transactions t ON t.txid = o.txid
                     LEFT JOIN blocks b ON b.hash = t.block_hash WHERE o.txid = ?1 AND o.vout = ?2",
                    params![input.txid.as_slice(), input.vout],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                ).optional()?;
                let Some((amount, eligible, coinbase, height, spent)) = previous else {
                    return Err(Error::InvalidData("missing pending input".into()));
                };
                if !eligible
                    || spent
                    || (coinbase && height.is_none_or(|height| spending_height - height < 100))
                {
                    return Err(Error::InvalidData(
                        "pending input is spent, ineligible, or immature".into(),
                    ));
                }
                input_total = input_total
                    .checked_add(amount)
                    .filter(|total| *total <= MAX_MONEY)
                    .ok_or_else(|| Error::InvalidData("pending input total overflow".into()))?;
                database.execute(
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
                output_total = output_total
                    .checked_add(output.amount)
                    .filter(|total| *total <= MAX_MONEY)
                    .ok_or_else(|| Error::InvalidData("pending output total overflow".into()))?;
                database.execute(
                    "INSERT INTO mempool_outputs VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        transaction.txid.as_slice(),
                        vout as i64,
                        output.amount,
                        output.script,
                        crate::store_block::utxo_eligible(&output.script, 1)
                    ],
                )?;
            }
            if transaction.inputs.is_empty()
                || transaction.outputs.is_empty()
                || input_total < output_total
            {
                return Err(Error::InvalidData(
                    "invalid pending transaction totals or counts".into(),
                ));
            }
        }
        database.execute("UPDATE sync_status SET ready = 1, observed_height = ?1, observed_hash = ?2,
            last_node_contact = ?3, pending_base_hash = ?2, pending_sequence = ?4, pending_observed_at = ?3,
            pending_revision = pending_revision + 1",
            params![base.height, base.hash.as_slice(), observed_at, sequence])?;
        database.commit()?;
        Ok(())
    }
}
