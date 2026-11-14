use rusqlite::{OptionalExtension, Transaction, params};

use crate::{Block, Error, Result};
use truenorth_rpc::{MAX_MONEY, Outpoint, Transaction as BlockTransaction};

pub(crate) fn insert_block_records(database: &Transaction<'_>, block: &Block) -> Result<()> {
    database.execute(
        "INSERT INTO blocks (hash, height, previous_hash, timestamp) VALUES (?1, ?2, ?3, ?4)",
        params![
            block.hash.as_slice(),
            block.height,
            block.previous_hash.as_ref().map(|hash| hash.as_slice()),
            block.timestamp
        ],
    )?;
    for (position, transaction) in block.transactions.iter().enumerate() {
        database.execute(
            "INSERT INTO transactions (txid, block_hash, position, is_coinbase) VALUES (?1, ?2, ?3, ?4)",
            params![transaction.txid.as_slice(), block.hash.as_slice(), position as i64, transaction.is_coinbase],
        )?;
        // Resolve inputs before inserting this transaction's outputs, so an
        // input cannot incorrectly resolve against its own transaction.
        let input_total = insert_spends(database, transaction, block.height)?;
        let mut output_total = 0_i64;
        for (vout, output) in transaction.outputs.iter().enumerate() {
            output_total = output_total
                .checked_add(output.amount)
                .ok_or_else(|| Error::InvalidData("output total overflow".into()))?;
            database.execute(
                "INSERT INTO outputs (txid, vout, amount, script, utxo_eligible) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![transaction.txid.as_slice(), vout as i64, output.amount,
                    output.script, utxo_eligible(&output.script, block.height)],
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

pub(crate) fn utxo_eligible(script: &[u8], block_height: i64) -> bool {
    // Core skips genesis outputs and excludes OP_RETURN or oversized scripts.
    block_height != 0 && script.first() != Some(&0x6a) && script.len() <= 10_000
}

fn insert_spends(
    database: &Transaction<'_>,
    transaction: &BlockTransaction,
    height: i64,
) -> Result<i64> {
    let mut total = 0_i64;
    for (vin, input) in transaction.inputs.iter().enumerate() {
        let amount = resolve_unspent_output(database, input, height)?;
        total = total
            .checked_add(amount)
            .filter(|total| *total <= MAX_MONEY)
            .ok_or_else(|| {
                Error::InvalidData("transaction input total exceeds MAX_MONEY".into())
            })?;
        database.execute(
            "INSERT INTO spends (spending_txid, vin, previous_txid, previous_vout) VALUES (?1, ?2, ?3, ?4)",
            params![transaction.txid.as_slice(), vin as i64, input.txid.as_slice(), input.vout],
        )?;
    }
    Ok(total)
}

fn resolve_unspent_output(
    database: &Transaction<'_>,
    input: &Outpoint,
    height: i64,
) -> Result<i64> {
    let previous: Option<(i64, bool, bool, i64, bool)> = database
        .query_row(
            "SELECT outputs.amount, outputs.utxo_eligible, transactions.is_coinbase, blocks.height,
            EXISTS (SELECT 1 FROM spends
                WHERE previous_txid = outputs.txid AND previous_vout = outputs.vout)
         FROM outputs
         JOIN transactions ON transactions.txid = outputs.txid
         JOIN blocks ON blocks.hash = transactions.block_hash
         WHERE outputs.txid = ?1 AND outputs.vout = ?2",
            params![input.txid.as_slice(), input.vout],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let outpoint = format!("{}:{}", hex::encode(input.txid), input.vout);
    let Some((amount, eligible, coinbase, created_height, spent)) = previous else {
        return Err(Error::InvalidData(format!("missing input {outpoint}")));
    };
    if !eligible || spent {
        return Err(Error::InvalidData(format!(
            "input {outpoint} is ineligible or already spent"
        )));
    }
    // TrueNorth v30.5.4 checks maturity at the spending block's height.
    if coinbase && height - created_height < 100 {
        return Err(Error::InvalidData(format!(
            "input {outpoint} is an immature coinbase"
        )));
    }
    Ok(amount)
}
