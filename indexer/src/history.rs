use rusqlite::params;
use serde::{Deserialize, Serialize};
use shared::{TransactionHistory, WalletTransaction};

use crate::address::address_from_script;
use crate::queries::{ReadSnapshot, read_optional_u64, read_u64};
use crate::{Error, Result, decode_hash};

#[derive(Deserialize, Serialize)]
struct Cursor {
    tip_hash: String,
    pending_revision: i64,
    height: i64,
    txid: String,
}

impl ReadSnapshot<'_> {
    pub fn query_transaction_history(
        &self,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<TransactionHistory> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidRequest(
                "history limit must be between 1 and 200".into(),
            ));
        }
        let revision: i64 =
            self.database
                .query_row("SELECT pending_revision FROM sync_status", [], |row| {
                    row.get(0)
                })?;
        let tip_hash = self
            .status
            .indexed_hash
            .clone()
            .ok_or_else(|| Error::Unavailable("indexed tip missing".into()))?;
        let cursor = decode_cursor(cursor)?;
        if cursor.as_ref().is_some_and(|cursor| {
            cursor.tip_hash != tip_hash || cursor.pending_revision != revision
        }) {
            return Err(Error::CursorChanged);
        }
        let last_height = cursor.as_ref().map(|cursor| cursor.height);
        let last_id = cursor
            .as_ref()
            .map(|cursor| decode_hash(&cursor.txid))
            .transpose()?;
        // Materialize only matching outpoints, then seek spends and transactions
        // by their indexes. CROSS JOIN preserves that lookup order in SQLite.
        let mut statement = self.database.prepare(
            "WITH matched_outputs AS MATERIALIZED (
                SELECT o.txid, o.vout FROM requested_scripts r
                CROSS JOIN outputs o ON o.script = r.script
                UNION ALL
                SELECT o.txid, o.vout FROM requested_scripts r
                CROSS JOIN mempool_outputs o ON o.script = r.script
             ), matching AS (
                SELECT txid FROM matched_outputs
                UNION
                SELECT s.spending_txid FROM matched_outputs o CROSS JOIN spends s
                    ON s.previous_txid = o.txid AND s.previous_vout = o.vout
                UNION
                SELECT s.spending_txid FROM matched_outputs o CROSS JOIN mempool_spends s
                    ON s.previous_txid = o.txid AND s.previous_vout = o.vout
             ), matched_transactions AS (
                SELECT t.txid, b.height AS block_height, b.timestamp
                FROM matching m CROSS JOIN transactions t ON t.txid = m.txid
                JOIN blocks b ON b.hash = t.block_hash
                UNION ALL
                SELECT t.txid, NULL, t.first_seen_at FROM matching m
                CROSS JOIN mempool_transactions t ON t.txid = m.txid
             ) SELECT txid, block_height, timestamp FROM matched_transactions
             WHERE ?1 IS NULL OR coalesce(block_height, 9223372036854775807) < ?1
                OR (coalesce(block_height, 9223372036854775807) = ?1 AND txid > ?2)
             ORDER BY coalesce(block_height, 9223372036854775807) DESC, txid
             LIMIT ?3",
        )?;
        let mut rows = statement.query(params![
            last_height,
            last_id.as_ref().map(|id| id.as_slice()),
            i64::from(limit) + 1
        ])?;
        let mut transactions = Vec::new();
        let mut last = None;
        let mut has_more = false;
        while let Some(row) = rows.next()? {
            if transactions.len() == usize::from(limit) {
                has_more = true;
                break;
            }
            let id: Vec<u8> = row.get(0)?;
            let height = read_optional_u64(row, 1)?;
            let timestamp = read_u64(row, 2)?;
            transactions.push(self.summarize_transaction(&id, height, timestamp)?);
            last = Some((height, hex::encode(id)));
        }
        let next_cursor = if has_more {
            let (height, txid) = last.ok_or_else(|| {
                Error::InvalidData("history cursor has no last transaction".into())
            })?;
            let cursor = Cursor {
                tip_hash,
                pending_revision: revision,
                height: height.map(|height| height as i64).unwrap_or(i64::MAX),
                txid,
            };
            let bytes = serde_json::to_vec(&cursor)
                .map_err(|_| Error::InvalidData("cannot encode history cursor".into()))?;
            Some(hex::encode(bytes))
        } else {
            None
        };
        Ok(TransactionHistory {
            transactions,
            next_cursor,
            index: self.status.clone(),
        })
    }

    fn summarize_transaction(
        &self,
        id: &[u8],
        height: Option<u64>,
        timestamp: u64,
    ) -> Result<WalletTransaction> {
        let mut received = 0_u64;
        let mut external = 0_u64;
        let mut output_total = 0_u64;
        let mut received_at = None;
        let mut sent_to = None;
        let mut outputs = self.database.prepare(
            "SELECT o.amount, o.script, r.address FROM all_outputs o
             LEFT JOIN requested_scripts r ON r.script = o.script WHERE o.txid = ?1 ORDER BY o.vout",
        )?;
        let mut rows = outputs.query([id])?;
        while let Some(row) = rows.next()? {
            let amount = read_u64(row, 0)?;
            let script: Vec<u8> = row.get(1)?;
            let address: Option<String> = row.get(2)?;
            output_total = add(output_total, amount)?;
            if let Some(address) = address {
                received = add(received, amount)?;
                if received_at.is_none() {
                    received_at = Some(address);
                }
            } else {
                external = add(external, amount)?;
                if sent_to.is_none() {
                    sent_to = address_from_script(&script, self.network);
                }
            }
        }
        let mut total_input = 0;
        let mut owned_input = 0;
        let mut all_owned = true;
        let mut has_owned_input = false;
        let mut inputs = self.database.prepare(
            "SELECT o.amount, EXISTS (SELECT 1 FROM requested_scripts r WHERE r.script = o.script)
             FROM all_spends s JOIN all_outputs o ON o.txid = s.previous_txid AND o.vout = s.previous_vout
             WHERE s.spending_txid = ?1",
        )?;
        for row in inputs.query_map([id], |row| Ok((read_u64(row, 0)?, row.get::<_, bool>(1)?)))? {
            let (amount, owned) = row?;
            total_input = add(total_input, amount)?;
            if owned {
                owned_input = add(owned_input, amount)?;
                has_owned_input = true;
            } else {
                all_owned = false;
            }
        }
        let mut fee = None;
        let mut sent_amount = 0;
        if has_owned_input {
            received_at = None;
            if all_owned {
                sent_amount = external;
                fee =
                    Some(total_input.checked_sub(output_total).ok_or_else(|| {
                        Error::InvalidData("history outputs exceed inputs".into())
                    })?);
            } else {
                // Mixed ownership cannot attribute the whole payment or fee to
                // this wallet. Show its net debit, with unknown fee/recipient.
                sent_amount = owned_input.saturating_sub(received);
                sent_to = None;
            }
        } else {
            sent_to = None;
        }
        let confirmations = match height {
            Some(height) => self
                .status
                .indexed_height
                .and_then(|tip| tip.checked_sub(height))
                .and_then(|difference| difference.checked_add(1))
                .ok_or_else(|| Error::InvalidData("history height exceeds indexed tip".into()))?,
            None => 0,
        };
        Ok(WalletTransaction {
            transaction_id: hex::encode(id),
            received_at,
            sent_to,
            received_amount: received,
            sent_amount,
            fee,
            block_height: height,
            confirmations,
            timestamp: Some(timestamp),
        })
    }
}

fn add(total: u64, amount: u64) -> Result<u64> {
    total
        .checked_add(amount)
        .ok_or_else(|| Error::InvalidData("history amount overflow".into()))
}

fn decode_cursor(text: Option<&str>) -> Result<Option<Cursor>> {
    let Some(text) = text else {
        return Ok(None);
    };
    if text.len() > 1024 {
        return Err(Error::InvalidRequest("history cursor is too long".into()));
    }
    let bytes =
        hex::decode(text).map_err(|_| Error::InvalidRequest("invalid history cursor".into()))?;
    let cursor: Cursor = serde_json::from_slice(&bytes)
        .map_err(|_| Error::InvalidRequest("invalid history cursor".into()))?;
    if cursor.height < 0
        || decode_hash(&cursor.txid).is_err()
        || decode_hash(&cursor.tip_hash).is_err()
    {
        return Err(Error::InvalidRequest(
            "invalid history cursor fields".into(),
        ));
    }
    Ok(Some(cursor))
}
