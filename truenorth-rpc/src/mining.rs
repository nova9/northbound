use serde_json::{Value, json};

use crate::{Error, NodeClient, Result, decode_hash};

// TrueNorth Core's MAX_BLOCK_SERIALIZED_SIZE is 4,000,000 bytes.
// Hexadecimal encoding uses two characters per byte.
pub const MAX_BLOCK_HEX_LENGTH: usize = 4_000_000 * 2;

pub fn validate_block_hex(block_hex: &str) -> Result<()> {
    // A serialized block contains at least its 80-byte header and transaction count.
    if block_hex.len() < 162 || block_hex.len() > MAX_BLOCK_HEX_LENGTH {
        return Err(Error::InvalidData(
            "block size is outside the permitted range".into(),
        ));
    }
    if block_hex.len() % 2 != 0 || !block_hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::InvalidData(
            "block must contain complete hexadecimal bytes".into(),
        ));
    }
    Ok(())
}

impl NodeClient {
    pub async fn fetch_block_template(&self) -> Result<Value> {
        let template = self
            .call_rpc("getblocktemplate", json!([{"rules": ["segwit"]}]))
            .await?;
        let id = template["longpollid"].as_str();
        if !template.is_object()
            || id.is_none_or(|id| id.is_empty() || id.len() > 128)
            || template["height"].as_u64().is_none()
            || !template["transactions"].is_array()
        {
            return Err(Error::InvalidData(
                "invalid mining template response".into(),
            ));
        }
        let parent = template["previousblockhash"]
            .as_str()
            .ok_or_else(|| Error::InvalidData("template has no parent hash".into()))?;
        decode_hash(parent)?;
        Ok(template)
    }

    /// None means accepted; strings preserve Core's rejection or duplicate result.
    pub async fn submit_block(&self, block_hex: &str) -> Result<Option<String>> {
        validate_block_hex(block_hex)?;
        let result = self
            .call_rpc_allow_null("submitblock", json!([block_hex]))
            .await?;
        match result {
            Value::Null => Ok(None),
            Value::String(reason) => Ok(Some(reason)),
            _ => Err(Error::InvalidData("invalid submitblock response".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_encoding_is_bounded_and_requires_complete_hexadecimal_bytes() {
        assert!(validate_block_hex(&"00".repeat(81)).is_ok());
        for invalid in [
            String::new(),
            "00".repeat(80),
            "z0".repeat(81),
            "0".repeat(163),
            "00".repeat(MAX_BLOCK_HEX_LENGTH / 2 + 1),
        ] {
            assert!(validate_block_hex(&invalid).is_err());
        }
    }
}
