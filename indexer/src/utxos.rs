use rusqlite::params;
use serde::{Deserialize, Serialize};
use shared::{MAX_UTXO_CURSOR_LENGTH, MAX_UTXO_PAGE_SIZE, UtxoResponse, WalletUtxo};

use crate::queries::{ReadSnapshot, read_u64};
use crate::{Error, Result, decode_hash};

#[derive(Deserialize, Serialize)]
struct Cursor {
    tip_hash: String,
    pending_revision: u64,
    block_height: u64,
    transaction_id: String,
    output_index: u32,
}

impl ReadSnapshot<'_> {
    pub fn query_utxos(&self, cursor: Option<&str>, limit: u16) -> Result<UtxoResponse> {
        if limit == 0 || limit > MAX_UTXO_PAGE_SIZE {
            return Err(Error::InvalidRequest(
                "UTXO limit must be between 1 and 10,000".into(),
            ));
        }
        let height = self
            .status
            .indexed_height
            .ok_or_else(|| Error::Unavailable("indexed tip is missing".into()))?;
        let tip_hash = self
            .status
            .indexed_hash
            .as_ref()
            .ok_or_else(|| Error::Unavailable("indexed tip hash is missing".into()))?;
        let pending_revision = self
            .status
            .pending_revision
            .ok_or_else(|| Error::Unavailable("pending snapshot revision is missing".into()))?;
        let cursor = decode_cursor(cursor)?;
        if cursor.as_ref().is_some_and(|cursor| {
            cursor.tip_hash != *tip_hash || cursor.pending_revision != pending_revision
        }) {
            return Err(Error::CursorChanged);
        }
        let last_height = cursor.as_ref().map(|cursor| cursor.block_height as i64);
        let last_txid = cursor
            .as_ref()
            .map(|cursor| decode_hash(&cursor.transaction_id))
            .transpose()?;
        let last_output = cursor.as_ref().map(|cursor| cursor.output_index);
        // Seek after the previous outpoint rather than skipping a growing OFFSET.
        // The small requested set stays first so SQLite uses its script index.
        let mut statement = self.database.prepare(
            "SELECT o.txid, o.vout, o.amount, o.script, b.height, t.is_coinbase
             FROM requested_scripts r CROSS JOIN outputs o ON o.script = r.script
             JOIN transactions t ON t.txid = o.txid JOIN blocks b ON b.hash = t.block_hash
             WHERE o.utxo_eligible = 1
               AND NOT EXISTS (SELECT 1 FROM spends WHERE previous_txid = o.txid AND previous_vout = o.vout)
               AND NOT EXISTS (SELECT 1 FROM mempool_spends WHERE previous_txid = o.txid AND previous_vout = o.vout)
               AND (?1 IS NULL OR (b.height, o.txid, o.vout) > (?1, ?2, ?3))
             ORDER BY b.height, o.txid, o.vout LIMIT ?4",
        )?;
        let mut rows = statement.query(params![
            last_height,
            last_txid.as_ref().map(|id| id.as_slice()),
            last_output,
            i64::from(limit) + 1
        ])?;
        let mut utxos = Vec::new();
        let mut has_more = false;
        while let Some(row) = rows.next()? {
            if utxos.len() == usize::from(limit) {
                has_more = true;
                break;
            }
            let txid: Vec<u8> = row.get(0)?;
            let script: Vec<u8> = row.get(3)?;
            let block_height = read_u64(row, 4)?;
            let confirmations = height
                .checked_sub(block_height)
                .and_then(|difference| difference.checked_add(1))
                .ok_or_else(|| Error::InvalidData("UTXO height exceeds indexed tip".into()))?;
            utxos.push(WalletUtxo {
                transaction_id: hex::encode(txid),
                output_index: row.get(1)?,
                amount: read_u64(row, 2)?,
                script_pubkey: hex::encode(script),
                block_height,
                confirmations,
                coinbase: row.get(5)?,
            });
        }
        let next_cursor = if has_more {
            let last = utxos
                .last()
                .ok_or_else(|| Error::InvalidData("UTXO page is empty".into()))?;
            let cursor = Cursor {
                tip_hash: tip_hash.clone(),
                pending_revision,
                block_height: last.block_height,
                transaction_id: last.transaction_id.clone(),
                output_index: last.output_index,
            };
            let bytes = serde_json::to_vec(&cursor)
                .map_err(|_| Error::InvalidData("cannot encode UTXO cursor".into()))?;
            Some(hex::encode(bytes))
        } else {
            None
        };
        Ok(UtxoResponse {
            utxos,
            next_cursor,
            index: self.status.clone(),
        })
    }
}

fn decode_cursor(text: Option<&str>) -> Result<Option<Cursor>> {
    let Some(text) = text else {
        return Ok(None);
    };
    if text.len() > MAX_UTXO_CURSOR_LENGTH {
        return Err(Error::InvalidRequest("UTXO cursor is too long".into()));
    }
    let bytes =
        hex::decode(text).map_err(|_| Error::InvalidRequest("invalid UTXO cursor".into()))?;
    let cursor: Cursor = serde_json::from_slice(&bytes)
        .map_err(|_| Error::InvalidRequest("invalid UTXO cursor".into()))?;
    if cursor.block_height > i64::MAX as u64
        || decode_hash(&cursor.transaction_id).is_err()
        || decode_hash(&cursor.tip_hash).is_err()
    {
        return Err(Error::InvalidRequest("invalid UTXO cursor fields".into()));
    }
    Ok(Some(cursor))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::test_support::{Fixture, block, hash, payment};
    use crate::{IndexReader, ScriptAddress};

    fn ready(fixture: &mut Fixture, sequence: u64) {
        let base = fixture.database.tip().unwrap().unwrap();
        fixture
            .database
            .publish_mempool_snapshot(&base, sequence, &[])
            .unwrap();
    }

    fn scripts() -> Vec<ScriptAddress> {
        vec![ScriptAddress {
            address: "test-miner".into(),
            script: vec![0x51],
        }]
    }

    #[test]
    fn retrieves_more_than_ten_thousand_outputs_at_one_address_without_truncation() {
        let mut fixture = Fixture::new(100);
        let mut funding = payment(2000, 1001, "0.0001");
        funding["vout"][0]["scriptPubKey"]["hex"] = json!("52");
        for output in 1..10_005 {
            funding["vout"].as_array_mut().unwrap().push(json!({
                "n": output,
                "value": serde_json::from_str::<serde_json::Number>("0.0001").unwrap(),
                "scriptPubKey": {"hex": "52"}
            }));
        }
        fixture
            .database
            .apply_block(&block(101, 0, vec![funding]))
            .unwrap();
        ready(&mut fixture, 1);
        let script = ScriptAddress {
            address: "busy-address".into(),
            script: vec![0x52],
        };
        let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
        let snapshot = reader.read_snapshot(&[script.clone(), script]).unwrap();
        let first = snapshot.query_utxos(None, MAX_UTXO_PAGE_SIZE).unwrap();
        assert_eq!(first.utxos.len(), usize::from(MAX_UTXO_PAGE_SIZE));
        assert!(first.next_cursor.is_some());
        let mut cursor = None;
        let mut outputs = Vec::new();
        loop {
            let page = snapshot.query_utxos(cursor.as_deref(), 1_000).unwrap();
            assert!(page.utxos.len() <= 1_000);
            outputs.extend(page.utxos);
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(outputs.len(), 10_005);
        for (number, output) in outputs.iter().enumerate() {
            assert_eq!(output.transaction_id, hex::encode(hash(2000)));
            assert_eq!(output.output_index, number as u32);
            assert_eq!(output.amount, 10_000);
            assert_eq!(output.script_pubkey, "52");
            assert_eq!(output.confirmations, 1);
            assert!(!output.coinbase);
        }
    }

    #[test]
    fn page_boundaries_match_complete_results_and_empty_pages_are_final() {
        let mut fixture = Fixture::new(4);
        ready(&mut fixture, 1);
        let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
        let snapshot = reader.read_snapshot(&scripts()).unwrap();
        let expected = snapshot
            .query_utxos(None, MAX_UTXO_PAGE_SIZE)
            .unwrap()
            .utxos;
        for limit in [1, 2, 4, 5] {
            let mut actual = Vec::new();
            let mut cursor = None;
            loop {
                let page = snapshot.query_utxos(cursor.as_deref(), limit).unwrap();
                assert!(!page.utxos.is_empty());
                assert_eq!(page.index.indexed_height, Some(4));
                actual.extend(page.utxos);
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
            assert_eq!(actual, expected);
        }
        drop(snapshot);
        let empty = reader
            .read_snapshot(&[])
            .unwrap()
            .query_utxos(None, 1)
            .unwrap();
        assert!(empty.utxos.is_empty());
        assert!(empty.next_cursor.is_none());
    }

    #[test]
    fn cursors_reject_pending_changes_growth_and_same_height_reorganizations() {
        for change in ["pending", "growth", "reorg"] {
            let mut fixture = Fixture::new(4);
            ready(&mut fixture, 1);
            let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
            let cursor = reader
                .read_snapshot(&scripts())
                .unwrap()
                .query_utxos(None, 1)
                .unwrap()
                .next_cursor
                .unwrap();
            if change == "growth" {
                fixture.database.apply_block(&block(5, 0, vec![])).unwrap();
            } else if change == "reorg" {
                let base = fixture.database.tip().unwrap().unwrap();
                fixture.database.rollback_tip(&base).unwrap();
                fixture.database.apply_block(&block(4, 1, vec![])).unwrap();
            }
            ready(&mut fixture, 2);
            let snapshot = reader.read_snapshot(&scripts()).unwrap();
            assert!(matches!(
                snapshot.query_utxos(Some(&cursor), 1),
                Err(Error::CursorChanged)
            ));
        }
    }

    #[test]
    fn rejects_malformed_cursors_and_invalid_page_limits() {
        let mut fixture = Fixture::new(4);
        ready(&mut fixture, 1);
        let mut reader = IndexReader::open(&fixture.path(), "regtest").unwrap();
        let snapshot = reader.read_snapshot(&scripts()).unwrap();
        for limit in [0, MAX_UTXO_PAGE_SIZE + 1] {
            assert!(matches!(
                snapshot.query_utxos(None, limit),
                Err(Error::InvalidRequest(_))
            ));
        }
        for cursor in [
            String::new(),
            "zz".into(),
            hex::encode(b"{}"),
            "a".repeat(MAX_UTXO_CURSOR_LENGTH + 1),
        ] {
            assert!(matches!(
                snapshot.query_utxos(Some(&cursor), 1),
                Err(Error::InvalidRequest(_))
            ));
        }
        let cursor = snapshot.query_utxos(None, 1).unwrap().next_cursor.unwrap();
        let valid: serde_json::Value =
            serde_json::from_slice(&hex::decode(cursor).unwrap()).unwrap();
        for (field, value) in [
            ("block_height", json!(u64::MAX)),
            ("transaction_id", json!("invalid")),
            ("tip_hash", json!("invalid")),
            ("output_index", json!(u64::MAX)),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            let cursor = hex::encode(serde_json::to_vec(&invalid).unwrap());
            assert!(matches!(
                snapshot.query_utxos(Some(&cursor), 1),
                Err(Error::InvalidRequest(_))
            ));
        }
    }
}
