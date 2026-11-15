use rusqlite::params;
use serde::{Deserialize, Serialize};
use shared::{TransactionHistory, WalletTransaction};

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
        let is_coinbase: bool = self.database.query_row(
            "SELECT is_coinbase FROM all_transactions WHERE txid = ?1",
            [id],
            |row| row.get(0),
        )?;
        let mut received = 0_u64;
        let mut output_total = 0_u64;
        let mut outputs = self.database.prepare(
            "SELECT o.amount, EXISTS (SELECT 1 FROM requested_scripts r WHERE r.script = o.script)
             FROM all_outputs o WHERE o.txid = ?1",
        )?;
        for row in outputs.query_map([id], |row| Ok((read_u64(row, 0)?, row.get::<_, bool>(1)?)))? {
            let (amount, owned) = row?;
            output_total = add(output_total, amount)?;
            if owned {
                received = add(received, amount)?;
            }
        }
        let mut total_input = 0;
        let mut owned_input = 0;
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
            }
        }
        // Keep credits and debits signed: clamping at zero loses contributions
        // when a client combines histories for different address batches.
        let net_amount = i64::try_from(i128::from(received) - i128::from(owned_input))
            .map_err(|_| Error::InvalidData("history net amount overflow".into()))?;
        let network_fee = if is_coinbase {
            None
        } else {
            Some(
                total_input
                    .checked_sub(output_total)
                    .ok_or_else(|| Error::InvalidData("history outputs exceed inputs".into()))?,
            )
        };
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
            net_amount,
            network_fee,
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
            assert_eq!(summaries[1].net_amount, -800_000_000);
            assert_eq!(summaries[2].net_amount, -500_000_000);
            assert_eq!(
                summaries[0].net_amount + summaries[1].net_amount,
                summaries[2].net_amount
            );
            assert_eq!(summaries[3].net_amount, 490_000_000);
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
        let coinbase = history
            .transactions
            .iter()
            .find(|tx| tx.transaction_id == hex::encode(hash(1101)))
            .unwrap();
        assert_eq!(coinbase.net_amount, 51_200_000_000);
        assert_eq!(coinbase.network_fee, None);
    }
}
