use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};

use crate::{Error, NodeClient, Result, atomic_units, decode_hash};

#[derive(Debug, Deserialize, Serialize)]
pub struct FeeEstimate {
    pub atomic_units_per_kvbyte: u64,
    pub target_blocks: u16,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransactionValidation {
    pub allowed: bool,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TransactionStatus {
    Unknown,
    Pending,
    Confirmed { block_height: u64 },
}

#[derive(Deserialize)]
struct Acceptance {
    allowed: bool,
    #[serde(rename = "reject-reason")]
    rejection_reason: Option<String>,
}

impl NodeClient {
    pub async fn estimate_fee_rate(&self) -> Result<FeeEstimate> {
        let estimate = self.call_rpc("estimatesmartfee", json!([6])).await?;
        let mempool = self.call_rpc("getmempoolinfo", json!([])).await?;
        let network = self.call_rpc("getnetworkinfo", json!([])).await?;
        let rate = select_fee_rate(&estimate, &mempool, &network)?;
        Ok(FeeEstimate {
            atomic_units_per_kvbyte: rate,
            target_blocks: 6,
        })
    }

    pub async fn test_transaction_acceptance(&self, hex: &str) -> Result<TransactionValidation> {
        validate_transaction_hex(hex)?;
        let value = self.call_rpc("testmempoolaccept", json!([[hex]])).await?;
        let mut results: Vec<Acceptance> = serde_json::from_value(value)
            .map_err(|_| Error::InvalidData("invalid mempool acceptance result".into()))?;
        if results.len() != 1 {
            return Err(Error::InvalidData(
                "expected one mempool acceptance result".into(),
            ));
        }
        let result = results.remove(0);
        Ok(TransactionValidation {
            allowed: result.allowed,
            rejection_reason: result.rejection_reason,
        })
    }

    pub async fn broadcast_transaction(&self, hex: &str) -> Result<String> {
        validate_transaction_hex(hex)?;
        self.fetch_tip().await?;
        let value = match self.call_rpc("sendrawtransaction", json!([hex])).await {
            Ok(value) => value,
            // The daemon recognizes this exact transaction in the active chain.
            // Decode it with TrueNorth to recover its ID, including its consensus rules.
            Err(Error::Remote { code: -27, .. }) => {
                return self.decode_transaction_id(hex).await;
            }
            Err(error) => return Err(error),
        };
        let txid = value.as_str().ok_or_else(|| {
            Error::InvalidData("broadcast did not return a transaction ID".into())
        })?;
        decode_hash(txid)?;
        Ok(txid.to_owned())
    }

    pub async fn decode_transaction_id(&self, hex: &str) -> Result<String> {
        validate_transaction_hex(hex)?;
        let decoded = self.call_rpc("decoderawtransaction", json!([hex])).await?;
        let txid = decoded["txid"]
            .as_str()
            .ok_or_else(|| Error::InvalidData("decoded transaction has no ID".into()))?;
        decode_hash(txid)?;
        Ok(txid.to_owned())
    }

    pub async fn fetch_transaction_status(&self, txid: &str) -> Result<TransactionStatus> {
        decode_hash(txid)?;
        let value = match self
            .call_rpc("getrawtransaction", json!([txid, true]))
            .await
        {
            Ok(value) => value,
            Err(Error::Remote {
                code: -5,
                ref message,
                ..
            }) if message.starts_with("No such mempool or blockchain transaction.") => {
                return Ok(TransactionStatus::Unknown);
            }
            // Missing/unsynced txindex cannot establish absence from the chain.
            Err(error) => return Err(error),
        };
        if value["txid"].as_str().map(decode_hash).transpose()? != Some(decode_hash(txid)?) {
            return Err(Error::InvalidData(
                "transaction lookup returned a different ID".into(),
            ));
        }
        if value.get("blockhash").is_none() && value.get("confirmations").is_none() {
            return Ok(TransactionStatus::Pending);
        }
        let block_hash = value["blockhash"]
            .as_str()
            .ok_or_else(|| Error::InvalidData("transaction has no block hash".into()))?;
        decode_hash(block_hash)?;
        if value["confirmations"]
            .as_u64()
            .is_none_or(|count| count == 0)
        {
            return Err(Error::InvalidData(
                "transaction is not in the active chain".into(),
            ));
        }
        let header = self.call_rpc("getblockheader", json!([block_hash])).await?;
        if header["hash"].as_str() != Some(block_hash)
            || header["confirmations"]
                .as_u64()
                .is_none_or(|count| count == 0)
        {
            return Err(Error::InvalidData(
                "transaction block is not in the active chain".into(),
            ));
        }
        let height = header["height"]
            .as_u64()
            .ok_or_else(|| Error::InvalidData("transaction block has no valid height".into()))?;
        Ok(TransactionStatus::Confirmed {
            block_height: height,
        })
    }
}

fn select_fee_rate(estimate: &Value, mempool: &Value, network: &Value) -> Result<u64> {
    let mut rate = 0;
    for (value, optional) in [
        (&estimate["feerate"], true),
        (&mempool["mempoolminfee"], false),
        (&network["relayfee"], false),
    ] {
        if optional && value.is_null() {
            continue;
        }
        let number: Number = serde_json::from_value(value.clone())
            .map_err(|_| Error::InvalidData("invalid fee rate".into()))?;
        // Core reports NORTH/kvB; converting to atomic units preserves its precision.
        let per_kvbyte = atomic_units(&number)? as u64;
        rate = rate.max(per_kvbyte);
    }
    if rate == 0 {
        return Err(Error::InvalidData("node returned a zero fee rate".into()));
    }
    Ok(rate)
}

pub fn validate_transaction_hex(hex: &str) -> Result<()> {
    if hex.is_empty()
        || hex.len() > 800_000
        || !hex.len().is_multiple_of(2)
        || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error::InvalidData(
            "signed transaction must be valid hexadecimal data of at most 800,000 characters"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{select_fee_rate, validate_transaction_hex};
    use serde_json::json;

    #[test]
    fn fee_rate_preserves_kvbyte_precision_and_respects_policy_floors() {
        for (estimate, mempool, relay, expected) in [
            ("0.00001001", "0.00001000", "0.00001000", 1001),
            ("0.00001001", "0.00001501", "0.00001000", 1501),
            ("0.00001001", "0.00001000", "0.00002001", 2001),
            ("null", "0.00001000", "1e-5", 1000),
        ] {
            assert_eq!(
                select_fee_rate(
                    &json!({"feerate": serde_json::from_str::<serde_json::Value>(estimate).unwrap()}),
                    &json!({"mempoolminfee": serde_json::from_str::<serde_json::Value>(mempool).unwrap()}),
                    &json!({"relayfee": serde_json::from_str::<serde_json::Value>(relay).unwrap()}),
                ).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn rejects_invalid_fee_rates_and_missing_policy_floors() {
        for value in [json!(-1), json!("invalid"), json!(0.000000001)] {
            assert!(
                select_fee_rate(
                    &json!({"feerate": value}),
                    &json!({"mempoolminfee": 0.00001}),
                    &json!({"relayfee": 0.00001})
                )
                .is_err()
            );
        }
        assert!(select_fee_rate(&json!({}), &json!({}), &json!({"relayfee": 0.00001})).is_err());
        assert!(
            select_fee_rate(
                &json!({}),
                &json!({"mempoolminfee": 0}),
                &json!({"relayfee": 0})
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_malformed_transaction_hex_before_rpc() {
        for hex in ["", "0", "zz", "00 11"] {
            assert!(validate_transaction_hex(hex).is_err());
        }
        assert!(validate_transaction_hex("0011aaff").is_ok());
    }
}
