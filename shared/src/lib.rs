use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ChainTip {
    pub height: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FeeEstimate {
    pub atomic_units_per_vbyte: u64,
    pub target_blocks: u16,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransactionHistoryRequest {
    pub addresses: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WalletTransaction {
    pub transaction_id: String,
    pub received_at: Option<String>,
    pub sent_to: Option<String>,
    pub received_amount: u64,
    pub sent_amount: u64,
    pub fee: Option<u64>,
    pub block_height: Option<u64>,
    pub confirmations: u64,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransactionHistory {
    pub transactions: Vec<WalletTransaction>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AddressActivityRequest {
    pub addresses: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AddressActivityResponse {
    pub used_addresses: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UtxoRequest {
    pub addresses: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WalletUtxo {
    pub transaction_id: String,
    pub output_index: u32,
    pub amount: u64,
    pub script_pubkey: String,
    pub block_height: u64,
    pub confirmations: u64,
    pub coinbase: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UtxoResponse {
    pub utxos: Vec<WalletUtxo>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SignedTransactionRequest {
    pub transaction_hex: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransactionValidation {
    pub allowed: bool,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BroadcastTransactionResponse {
    pub transaction_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransactionStatusRequest {
    pub transaction_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransactionStatusResponse {
    pub known: bool,
}

#[cfg(test)]
mod tests {
    use super::ChainTip;

    #[test]
    fn chain_tip_uses_the_expected_json_format() {
        let chain_tip = ChainTip { height: 12 };

        let json = serde_json::to_string(&chain_tip).expect("test chain tip should serialize");

        assert_eq!(json, r#"{"height":12}"#);
    }

    #[test]
    fn chain_tip_can_be_read_from_json() {
        let json = r#"{"height":12}"#;

        let chain_tip: ChainTip =
            serde_json::from_str(json).expect("valid chain tip JSON should deserialize");

        assert_eq!(chain_tip.height, 12);
    }
}
