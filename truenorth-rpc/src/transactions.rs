use serde::{Deserialize, Serialize};
use serde_json::{Number, json};

use crate::{Error, NodeClient, Result, atomic_units, decode_hash};

#[derive(Debug, Deserialize, Serialize)]
pub struct FeeEstimate {
    pub atomic_units_per_vbyte: u64,
    pub target_blocks: u16,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransactionValidation {
    pub allowed: bool,
    pub rejection_reason: Option<String>,
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
            rate = rate.max(round_fee_rate(&number)?);
        }
        if rate == 0 {
            return Err(Error::InvalidData("node returned a zero fee rate".into()));
        }
        Ok(FeeEstimate {
            atomic_units_per_vbyte: rate,
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
        let value = self.call_rpc("sendrawtransaction", json!([hex])).await?;
        let txid = value.as_str().ok_or_else(|| {
            Error::InvalidData("broadcast did not return a transaction ID".into())
        })?;
        decode_hash(txid)?;
        Ok(txid.to_owned())
    }

    pub async fn is_transaction_known(&self, txid: &str) -> Result<bool> {
        decode_hash(txid)?;
        match self
            .call_rpc("getrawtransaction", json!([txid, false]))
            .await
        {
            Ok(_) => Ok(true),
            Err(Error::Remote { code: -5, .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

fn round_fee_rate(number: &Number) -> Result<u64> {
    let per_kilovbyte = atomic_units(number)?;
    let rounded = per_kilovbyte
        .checked_add(999)
        .ok_or_else(|| Error::InvalidData("fee rate overflow".into()))?
        / 1000;
    Ok(rounded as u64)
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
    use super::{round_fee_rate, validate_transaction_hex};

    #[test]
    fn fee_rate_rounds_up_without_floating_point() {
        for (value, expected) in [("0.00001000", 1), ("0.00001001", 2), ("1e-5", 1)] {
            assert_eq!(
                round_fee_rate(&serde_json::from_str(value).unwrap()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn rejects_malformed_transaction_hex_before_rpc() {
        for hex in ["", "0", "zz", "00 11"] {
            assert!(validate_transaction_hex(hex).is_err());
        }
        assert!(validate_transaction_hex("0011aaff").is_ok());
    }
}
