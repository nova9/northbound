use std::collections::BTreeSet;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use shared::{TransactionHistory, WalletTransaction};

use crate::queries::{
    ReadSnapshot, calculate_confirmations, decode_cursor, encode_cursor, read_optional_u64,
    read_u64,
};
use crate::{Error, Result, decode_hash};

const MAX_HISTORY_PAGE_SIZE: u16 = 200;
const MAX_HISTORY_CURSOR_LENGTH: usize = 1024;

// Pending transactions have `block_height = NULL`. Mapping `NULL` to `i64::MAX`
// sorts unconfirmed transactions above the newest confirmed block.
const PENDING_SORT_HEIGHT: i64 = i64::MAX;

#[derive(Deserialize, Serialize)]
struct HistoryCursor {
    tip_hash: String,
    pending_revision: u64,
    height: i64,
    txid: String,
}

impl ReadSnapshot<'_> {
    /// Returns one page of confirmed and pending transactions touching the requested
    /// scripts, ordered newest-first (pending transactions first, then descending block height).
    pub fn query_transaction_history(
        &self,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<TransactionHistory> {
        if !(1..=MAX_HISTORY_PAGE_SIZE).contains(&limit) {
            return Err(Error::InvalidRequest(
                "history limit must be between 1 and 200".into(),
            ));
        }

        let (tip_height, tip_hash, pending_revision) = self.snapshot_identity()?;
        let cursor = parse_history_cursor(cursor, tip_hash, pending_revision)?;

        let (last_height, last_txid) = match &cursor {
            Some(cursor) => (Some(cursor.height), Some(decode_hash(&cursor.txid)?)),
            None => (None, None),
        };

        // 1. Find every confirmed/pending output belonging to our requested scripts.
        // 2. Collect transactions that created those outputs or spent them.
        // 3. Look up their height, timestamp, and coinbase flag, then paginate.
        // `CROSS JOIN` keeps SQLite seeking from the small `requested_scripts` table.
        let mut statement = self.database.prepare(
            "WITH matched_outputs AS MATERIALIZED (
                 SELECT o.txid, o.vout
                 FROM requested_scripts r
                 CROSS JOIN outputs o ON o.script = r.script
                 UNION ALL
                 SELECT o.txid, o.vout
                 FROM requested_scripts r
                 CROSS JOIN mempool_outputs o ON o.script = r.script
             ),
             matching_txids AS (
                 SELECT txid FROM matched_outputs
                 UNION
                 SELECT s.spending_txid
                 FROM matched_outputs o
                 CROSS JOIN spends s
                     ON s.previous_txid = o.txid AND s.previous_vout = o.vout
                 UNION
                 SELECT s.spending_txid
                 FROM matched_outputs o
                 CROSS JOIN mempool_spends s
                     ON s.previous_txid = o.txid AND s.previous_vout = o.vout
             ),
             matched_transactions AS (
                 SELECT t.txid, b.height AS block_height, b.timestamp, t.is_coinbase
                 FROM matching_txids m
                 CROSS JOIN transactions t ON t.txid = m.txid
                 JOIN blocks b ON b.hash = t.block_hash
                 UNION ALL
                 SELECT t.txid, NULL, t.first_seen_at, 0
                 FROM matching_txids m
                 CROSS JOIN mempool_transactions t ON t.txid = m.txid
             )
             SELECT txid, block_height, timestamp, is_coinbase
             FROM matched_transactions
             WHERE ?1 IS NULL
                OR coalesce(block_height, ?2) < ?1
                OR (coalesce(block_height, ?2) = ?1 AND txid > ?3)
             ORDER BY coalesce(block_height, ?2) DESC, txid
             LIMIT ?4",
        )?;

        let mut rows = statement.query(params![
            last_height,
            PENDING_SORT_HEIGHT,
            last_txid.as_ref().map(|id| id.as_slice()),
            i64::from(limit) + 1
        ])?;

        let mut transactions = Vec::new();
        let mut has_more = false;

        while let Some(row) = rows.next()? {
            if transactions.len() == usize::from(limit) {
                has_more = true;
                break;
            }

            let txid: Vec<u8> = row.get(0)?;
            let block_height = read_optional_u64(row, 1)?;
            let timestamp = read_u64(row, 2)?;
            let is_coinbase: bool = row.get(3)?;

            transactions.push(self.summarize_transaction(
                &txid,
                block_height,
                timestamp,
                is_coinbase,
                tip_height,
            )?);
        }

        let next_cursor = if has_more {
            let last = transactions.last().ok_or_else(|| {
                Error::InvalidData("history cursor has no last transaction".into())
            })?;
            let next = HistoryCursor {
                tip_hash: tip_hash.to_owned(),
                pending_revision,
                height: last
                    .block_height
                    .map(|height| height as i64)
                    .unwrap_or(PENDING_SORT_HEIGHT),
                txid: last.transaction_id.clone(),
            };
            Some(encode_cursor(&next, "history")?)
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
        txid: &[u8],
        block_height: Option<u64>,
        timestamp: u64,
        is_coinbase: bool,
        tip_height: u64,
    ) -> Result<WalletTransaction> {
        let (total_output, owned_output, received_addresses) =
            self.sum_transaction_outputs(txid)?;
        let (total_input, owned_input) = self.sum_transaction_inputs(txid)?;

        // Signed net balance change for the requested scripts (`owned_output - owned_input`).
        // Keeping this signed lets the wallet add batch results together accurately.
        let net_amount = i64::try_from(i128::from(owned_output) - i128::from(owned_input))
            .map_err(|_| Error::InvalidData("history net amount overflow".into()))?;

        // Fee covers the entire transaction, regardless of which addresses own inputs/outputs.
        let network_fee = if is_coinbase {
            None
        } else {
            let fee = total_input
                .checked_sub(total_output)
                .ok_or_else(|| Error::InvalidData("history outputs exceed inputs".into()))?;
            Some(fee)
        };

        let confirmations = match block_height {
            Some(height) => calculate_confirmations(tip_height, height, "history")?,
            None => 0,
        };

        Ok(WalletTransaction {
            transaction_id: hex::encode(txid),
            net_amount,
            network_fee,
            coinbase: is_coinbase,
            block_height,
            confirmations,
            timestamp: Some(timestamp),
            received_addresses,
        })
    }

    /// Returns `(total_output, owned_output, received_addresses)` across all outputs of `txid`.
    fn sum_transaction_outputs(&self, txid: &[u8]) -> Result<(u64, u64, Vec<String>)> {
        let mut statement = self.database.prepare(
            "SELECT o.amount, r.address
             FROM all_outputs o
             LEFT JOIN requested_scripts r ON r.script = o.script
             WHERE o.txid = ?1",
        )?;

        let mut total = 0_u64;
        let mut owned_total = 0_u64;
        let mut received_addresses = BTreeSet::new();
        let rows = statement.query_map([txid], |row| {
            Ok((read_u64(row, 0)?, row.get::<_, Option<String>>(1)?))
        })?;

        for row in rows {
            let (amount, owned_address) = row?;
            total = add_amount(total, amount)?;
            if let Some(address) = owned_address {
                owned_total = add_amount(owned_total, amount)?;
                received_addresses.insert(address);
            }
        }

        Ok((total, owned_total, received_addresses.into_iter().collect()))
    }

    /// Returns `(total_input, owned_input)` across all inputs spent by `txid`.
    fn sum_transaction_inputs(&self, txid: &[u8]) -> Result<(u64, u64)> {
        let mut statement = self.database.prepare(
            "SELECT o.amount,
                    EXISTS (SELECT 1 FROM requested_scripts r WHERE r.script = o.script)
             FROM all_spends s
             JOIN all_outputs o
                 ON o.txid = s.previous_txid AND o.vout = s.previous_vout
             WHERE s.spending_txid = ?1",
        )?;

        let mut total = 0_u64;
        let mut owned_total = 0_u64;
        let rows = statement.query_map([txid], |row| {
            Ok((read_u64(row, 0)?, row.get::<_, bool>(1)?))
        })?;

        for row in rows {
            let (amount, is_owned) = row?;
            total = add_amount(total, amount)?;
            if is_owned {
                owned_total = add_amount(owned_total, amount)?;
            }
        }

        Ok((total, owned_total))
    }
}

fn add_amount(total: u64, amount: u64) -> Result<u64> {
    total
        .checked_add(amount)
        .ok_or_else(|| Error::InvalidData("history amount overflow".into()))
}

fn parse_history_cursor(
    text: Option<&str>,
    expected_tip_hash: &str,
    expected_revision: u64,
) -> Result<Option<HistoryCursor>> {
    let Some(cursor) = decode_cursor::<HistoryCursor>(text, MAX_HISTORY_CURSOR_LENGTH, "history")?
    else {
        return Ok(None);
    };

    if cursor.height < 0
        || decode_hash(&cursor.txid).is_err()
        || decode_hash(&cursor.tip_hash).is_err()
    {
        return Err(Error::InvalidRequest(
            "invalid history cursor fields".into(),
        ));
    }

    if cursor.tip_hash != expected_tip_hash || cursor.pending_revision != expected_revision {
        return Err(Error::CursorChanged);
    }

    Ok(Some(cursor))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_support::{Fixture, block, hash, payment};
    use crate::{IndexReader, ScriptAddress};

    #[test]
    fn mixed_input_batches_preserve_net_amount_and_repeat_the_whole_network_fee() {
        for pending in [false, true] {
            let mut fixture = Fixture::new(100);
            let mut funding = payment(2000, 1001, "2");
            funding["vout"][0]["scriptPubKey"]["hex"] = json!("52");
            funding["vout"].as_array_mut().unwrap().push(json!({
                "n": 1, "value": 8, "scriptPubKey": {"hex": "53"}
            }));
            fixture
                .database
                .apply_block(&block(101, 0, vec![funding]))
                .unwrap();
            let mut spend = payment(2001, 2000, "5");
            spend["vout"][0]["scriptPubKey"]["hex"] = json!("54");
            spend["vin"].as_array_mut().unwrap().push(json!({
                "txid": hex::encode(hash(2000)), "vout": 1
            }));
            spend["vout"].as_array_mut().unwrap().push(json!({
                "n": 1, "value": serde_json::from_str::<serde_json::Number>("4.9").unwrap(),
                "scriptPubKey": {"hex": "55"}
            }));
            if pending {
                let transaction = truenorth_rpc::decode_transaction(&spend.to_string()).unwrap();
                let base = fixture.database.tip().unwrap().unwrap();
                fixture
                    .database
                    .publish_mempool_snapshot(&base, 1, &[transaction])
                    .unwrap();
            } else {
                fixture
                    .database
                    .apply_block(&block(102, 0, vec![spend]))
                    .unwrap();
                let base = fixture.database.tip().unwrap().unwrap();
                fixture
                    .database
                    .publish_mempool_snapshot(&base, 1, &[])
                    .unwrap();
            }
            let a = ScriptAddress {
                address: "A".into(),
                script: vec![0x52],
            };
            let b = ScriptAddress {
                address: "B".into(),
                script: vec![0x53],
            };
            let c = ScriptAddress {
                address: "C".into(),
                script: vec![0x54],
            };
            let external = ScriptAddress {
                address: "recipient".into(),
                script: vec![0x55],
            };
            let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
            let mut summaries = Vec::new();
            for addresses in [
                vec![a.clone(), c.clone()],
                vec![b.clone()],
                vec![a, b, c],
                vec![external],
            ] {
                let snapshot = reader.read_snapshot(&addresses).unwrap();
                let history = snapshot.query_transaction_history(None, 100).unwrap();
                summaries.push(
                    history
                        .transactions
                        .into_iter()
                        .find(|transaction| transaction.transaction_id == hex::encode(hash(2001)))
                        .unwrap(),
                );
            }
            assert_eq!(summaries[0].net_amount, 300_000_000);
            assert_eq!(summaries[0].received_addresses, vec!["C"]);
            assert_eq!(summaries[1].net_amount, -800_000_000);
            assert!(summaries[1].received_addresses.is_empty());
            assert_eq!(summaries[2].net_amount, -500_000_000);
            assert_eq!(summaries[2].received_addresses, vec!["C"]);
            assert_eq!(
                summaries[0].net_amount + summaries[1].net_amount,
                summaries[2].net_amount
            );
            assert_eq!(summaries[3].net_amount, 490_000_000);
            assert_eq!(summaries[3].received_addresses, vec!["recipient"]);
            for summary in summaries {
                assert_eq!(summary.network_fee, Some(10_000_000));
                assert_eq!(summary.confirmations, if pending { 0 } else { 1 });
            }
        }
    }

    #[test]
    fn coinbase_has_no_fee_and_self_payment_debits_only_the_fee() {
        let mut fixture = Fixture::new(100);
        fixture
            .database
            .apply_block(&block(101, 0, vec![payment(2000, 1001, "511.99999837")]))
            .unwrap();
        let base = fixture.database.tip().unwrap().unwrap();
        fixture
            .database
            .publish_mempool_snapshot(&base, 1, &[])
            .unwrap();
        let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
        let script = ScriptAddress {
            address: "wallet".into(),
            script: vec![0x51],
        };
        let snapshot = reader.read_snapshot(&[script]).unwrap();
        let history = snapshot.query_transaction_history(None, 200).unwrap();
        let self_payment = history
            .transactions
            .iter()
            .find(|tx| tx.transaction_id == hex::encode(hash(2000)))
            .unwrap();
        assert_eq!(self_payment.net_amount, -163);
        assert_eq!(self_payment.network_fee, Some(163));
        assert!(!self_payment.coinbase);
        let coinbase = history
            .transactions
            .iter()
            .find(|tx| tx.transaction_id == hex::encode(hash(1101)))
            .unwrap();
        assert_eq!(coinbase.net_amount, 51_200_000_000);
        assert_eq!(coinbase.network_fee, None);
        assert!(coinbase.coinbase);
        assert_eq!(coinbase.received_addresses, vec!["wallet"]);
    }
}
