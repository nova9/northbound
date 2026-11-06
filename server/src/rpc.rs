use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use shared::{FeeEstimate, TransactionValidation, WalletTransaction, WalletUtxo};

const MAX_ADDRESSES_PER_REQUEST: usize = 100;
const MAX_TRANSACTION_HEX_LENGTH: usize = 800_000;
const FEE_TARGET_BLOCKS: u16 = 6;
const VBYTES_PER_KILOVBYTE: u64 = 1_000;

#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    id: &'a str,
    method: &'a str,
    params: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize)]
struct BlockchainInfo {
    blocks: u64,
    chain: String,
}

#[derive(Debug, Deserialize)]
struct SmartFeeEstimate {
    feerate: Option<Number>,
}

#[derive(Debug, Deserialize)]
struct MempoolInfo {
    mempoolminfee: Number,
}

#[derive(Debug, Deserialize)]
struct NetworkInfo {
    relayfee: Number,
}

#[derive(Debug, Deserialize)]
struct ScanBlocksResult {
    relevant_blocks: Vec<String>,
    completed: bool,
}

#[derive(Debug, Deserialize)]
struct ScanTxOutSetResult {
    success: bool,
    unspents: Vec<ScanTxOutSetEntry>,
}

#[derive(Debug, Deserialize)]
struct ScanTxOutSetEntry {
    txid: String,
    vout: u32,
    #[serde(rename = "scriptPubKey")]
    script_pubkey: String,
    amount: Number,
    coinbase: bool,
    height: u64,
    confirmations: u64,
}

#[derive(Debug, Deserialize)]
struct DescriptorActivityResult {
    activity: Vec<DescriptorActivity>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum DescriptorActivity {
    #[serde(rename = "receive")]
    Receive {
        amount: Number,
        height: Option<u64>,
        txid: String,
        output_spk: ScriptPubKey,
    },
    #[serde(rename = "spend")]
    Spend {
        amount: Number,
        blockhash: Option<String>,
        height: Option<u64>,
        spend_txid: String,
        prevout_spk: Option<ScriptPubKey>,
    },
}

#[derive(Debug, Deserialize)]
struct ScriptPubKey {
    address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTransaction {
    vin: Vec<Value>,
    vout: Vec<RawTransactionOutput>,
}

#[derive(Debug, Deserialize)]
struct RawTransactionOutput {
    value: Number,
    #[serde(rename = "scriptPubKey")]
    script_pubkey: ScriptPubKey,
}

struct ActivitySummary {
    transaction_id: String,
    received_amount: u64,
    received_at: Option<String>,
    spent_amount: u64,
    spent_input_count: usize,
    block_height: Option<u64>,
    block_hash: Option<String>,
    confirmations: u64,
}

#[derive(Debug, Deserialize)]
struct MempoolAcceptance {
    allowed: bool,
    #[serde(rename = "reject-reason")]
    rejection_reason: Option<String>,
}

pub struct NodeRpcClient {
    url: String,
    cookie_path: PathBuf,
    http_client: reqwest::Client,
    expected_network: String,
}

impl NodeRpcClient {
    pub fn new(url: String, cookie_path: PathBuf, expected_network: String) -> Self {
        Self {
            url,
            cookie_path,
            http_client: reqwest::Client::new(),
            expected_network,
        }
    }

    pub async fn get_block_height(&self) -> Result<u64, String> {
        let blockchain_info = self
            .call_rpc::<BlockchainInfo>("chain-tip", "getblockchaininfo", Vec::new())
            .await?;

        if blockchain_info.chain != self.expected_network {
            return Err(format!(
                "TrueNorth node is on {}, but the server is configured for {}",
                blockchain_info.chain, self.expected_network
            ));
        }

        Ok(blockchain_info.blocks)
    }

    pub async fn estimate_fee_rate(&self) -> Result<FeeEstimate, String> {
        let smart_fee = self
            .call_rpc::<SmartFeeEstimate>(
                "fee-estimate",
                "estimatesmartfee",
                vec![json!(FEE_TARGET_BLOCKS)],
            )
            .await?;
        let mempool_info = self
            .call_rpc::<MempoolInfo>("mempool-fee-floor", "getmempoolinfo", Vec::new())
            .await?;
        let network_info = self
            .call_rpc::<NetworkInfo>("relay-fee-floor", "getnetworkinfo", Vec::new())
            .await?;

        let candidate_rates = [
            smart_fee.feerate.as_ref(),
            Some(&mempool_info.mempoolminfee),
            Some(&network_info.relayfee),
        ];
        let mut atomic_units_per_vbyte = 0;

        for rate in candidate_rates.into_iter().flatten() {
            atomic_units_per_vbyte = atomic_units_per_vbyte.max(fee_rate_to_atomic_units(rate)?);
        }

        if atomic_units_per_vbyte == 0 {
            return Err("TrueNorth node returned a zero fee rate".to_owned());
        }

        Ok(FeeEstimate {
            atomic_units_per_vbyte,
            target_blocks: FEE_TARGET_BLOCKS,
        })
    }

    pub async fn get_used_addresses(&self, addresses: &[String]) -> Result<Vec<String>, String> {
        validate_addresses(addresses)?;

        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let descriptors = address_descriptors(addresses);
        let scan_result = self
            .call_rpc::<ScanBlocksResult>(
                "address-activity-block-scan",
                "scanblocks",
                vec![json!("start"), json!(descriptors), json!(0)],
            )
            .await?;

        if !scan_result.completed {
            return Err("TrueNorth address activity scan was interrupted".to_owned());
        }

        if scan_result.relevant_blocks.is_empty() {
            return Ok(Vec::new());
        }

        let activity_result = self
            .call_rpc::<DescriptorActivityResult>(
                "address-activity",
                "getdescriptoractivity",
                vec![
                    json!(scan_result.relevant_blocks),
                    json!(descriptors),
                    json!(true),
                ],
            )
            .await?;

        let requested_addresses = addresses.iter().cloned().collect::<HashSet<_>>();
        let mut used_addresses = HashSet::new();

        for activity in activity_result.activity {
            let address = match activity {
                DescriptorActivity::Receive { output_spk, .. } => output_spk.address,
                DescriptorActivity::Spend { prevout_spk, .. } => {
                    prevout_spk.and_then(|script| script.address)
                }
            };

            if let Some(address) = address
                && requested_addresses.contains(&address)
            {
                used_addresses.insert(address);
            }
        }

        let mut used_addresses = used_addresses.into_iter().collect::<Vec<_>>();
        used_addresses.sort();

        Ok(used_addresses)
    }

    pub async fn get_transactions_for_addresses(
        &self,
        addresses: &[String],
    ) -> Result<Vec<WalletTransaction>, String> {
        validate_addresses(addresses)?;

        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let descriptors = address_descriptors(addresses);

        let scan_result = self
            .call_rpc::<ScanBlocksResult>(
                "transaction-block-scan",
                "scanblocks",
                vec![json!("start"), json!(descriptors), json!(0)],
            )
            .await?;

        if !scan_result.completed {
            return Err("TrueNorth block scan was interrupted".to_owned());
        }

        let activity_result = self
            .call_rpc::<DescriptorActivityResult>(
                "transaction-activity",
                "getdescriptoractivity",
                vec![
                    json!(scan_result.relevant_blocks),
                    json!(descriptors),
                    json!(true),
                ],
            )
            .await?;

        let chain_height = self.get_block_height().await?;

        self.wallet_transactions(activity_result.activity, addresses, chain_height)
            .await
    }

    pub async fn get_utxos_for_addresses(
        &self,
        addresses: &[String],
    ) -> Result<Vec<WalletUtxo>, String> {
        validate_addresses(addresses)?;

        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let descriptors = address_descriptors(addresses);

        let scan_result = self
            .call_rpc::<ScanTxOutSetResult>(
                "wallet-utxo-scan",
                "scantxoutset",
                vec![json!("start"), json!(descriptors)],
            )
            .await?;

        if !scan_result.success {
            return Err("TrueNorth UTXO scan was interrupted".to_owned());
        }

        let mut utxos = Vec::with_capacity(scan_result.unspents.len());

        for unspent in scan_result.unspents {
            utxos.push(WalletUtxo {
                transaction_id: unspent.txid,
                output_index: unspent.vout,
                amount: north_to_atomic_units(&unspent.amount)?,
                script_pubkey: unspent.script_pubkey,
                block_height: unspent.height,
                confirmations: unspent.confirmations,
                coinbase: unspent.coinbase,
            });
        }

        Ok(utxos)
    }

    pub async fn test_transaction_acceptance(
        &self,
        transaction_hex: &str,
    ) -> Result<TransactionValidation, String> {
        validate_transaction_hex(transaction_hex)?;

        let mut results = self
            .call_rpc::<Vec<MempoolAcceptance>>(
                "test-transaction-acceptance",
                "testmempoolaccept",
                vec![json!([transaction_hex])],
            )
            .await?;

        if results.len() != 1 {
            return Err(
                "TrueNorth returned an unexpected transaction validation result".to_owned(),
            );
        }

        let result = results.remove(0);

        Ok(TransactionValidation {
            allowed: result.allowed,
            rejection_reason: result.rejection_reason,
        })
    }

    pub async fn broadcast_transaction(&self, transaction_hex: &str) -> Result<String, String> {
        validate_transaction_hex(transaction_hex)?;

        self.call_rpc::<String>(
            "broadcast-transaction",
            "sendrawtransaction",
            vec![json!(transaction_hex)],
        )
        .await
    }

    pub async fn is_transaction_known(&self, transaction_id: &str) -> Result<bool, String> {
        validate_transaction_id(transaction_id)?;

        let result = self
            .call_rpc_allow_not_found::<String>(
                "transaction-status",
                "getrawtransaction",
                vec![json!(transaction_id), json!(false)],
            )
            .await?;

        Ok(result.is_some())
    }

    async fn wallet_transactions(
        &self,
        activity: Vec<DescriptorActivity>,
        wallet_addresses: &[String],
        chain_height: u64,
    ) -> Result<Vec<WalletTransaction>, String> {
        let summaries = summarize_activity(activity, chain_height)?;
        let wallet_addresses = wallet_addresses.iter().cloned().collect::<HashSet<_>>();
        let mut transactions = Vec::with_capacity(summaries.len());

        for summary in summaries {
            if summary.spent_amount == 0 {
                transactions.push(received_transaction(summary));
                continue;
            }

            let mut params = vec![json!(summary.transaction_id), json!(true)];

            if let Some(block_hash) = &summary.block_hash {
                params.push(json!(block_hash));
            }

            let raw_transaction = self
                .call_rpc::<RawTransaction>(
                    "outgoing-transaction-details",
                    "getrawtransaction",
                    params,
                )
                .await?;
            transactions.push(sent_transaction(
                summary,
                raw_transaction,
                &wallet_addresses,
            )?);
        }

        sort_transactions_newest_first(&mut transactions);

        Ok(transactions)
    }

    async fn call_rpc<T>(
        &self,
        request_id: &str,
        method: &str,
        params: Vec<Value>,
    ) -> Result<T, String>
    where
        T: DeserializeOwned,
    {
        let cookie = tokio::fs::read_to_string(&self.cookie_path)
            .await
            .map_err(|error| format!("could not read TrueNorth RPC cookie: {error}"))?;

        let Some((username, password)) = cookie.trim_end().split_once(':') else {
            return Err("TrueNorth RPC cookie has an invalid format".to_owned());
        };

        let request = RpcRequest {
            jsonrpc: "2.0",
            id: request_id,
            method,
            params,
        };

        let response = self
            .http_client
            .post(&self.url)
            .basic_auth(username, Some(password))
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("could not contact TrueNorth RPC: {error}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "TrueNorth RPC returned HTTP status {}",
                response.status()
            ));
        }

        let rpc_response = response
            .json::<RpcResponse<T>>()
            .await
            .map_err(|error| format!("TrueNorth RPC returned invalid JSON: {error}"))?;

        if let Some(error) = rpc_response.error {
            return Err(format!(
                "TrueNorth RPC error {}: {}",
                error.code, error.message
            ));
        }

        rpc_response
            .result
            .ok_or_else(|| "TrueNorth RPC response did not contain a result".to_owned())
    }

    async fn call_rpc_allow_not_found<T>(
        &self,
        request_id: &str,
        method: &str,
        params: Vec<Value>,
    ) -> Result<Option<T>, String>
    where
        T: DeserializeOwned,
    {
        let cookie = tokio::fs::read_to_string(&self.cookie_path)
            .await
            .map_err(|error| format!("could not read TrueNorth RPC cookie: {error}"))?;
        let Some((username, password)) = cookie.trim_end().split_once(':') else {
            return Err("TrueNorth RPC cookie has an invalid format".to_owned());
        };
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: request_id,
            method,
            params,
        };
        let response = self
            .http_client
            .post(&self.url)
            .basic_auth(username, Some(password))
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("could not contact TrueNorth RPC: {error}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "TrueNorth RPC returned HTTP status {}",
                response.status()
            ));
        }

        let rpc_response = response
            .json::<RpcResponse<T>>()
            .await
            .map_err(|error| format!("TrueNorth RPC returned invalid JSON: {error}"))?;

        if let Some(error) = rpc_response.error {
            if error.code == -5 {
                return Ok(None);
            }

            return Err(format!(
                "TrueNorth RPC error {}: {}",
                error.code, error.message
            ));
        }

        rpc_response
            .result
            .map(Some)
            .ok_or_else(|| "TrueNorth RPC response did not contain a result".to_owned())
    }
}

fn sort_transactions_newest_first(transactions: &mut [WalletTransaction]) {
    transactions.sort_by(
        |left, right| match (left.block_height, right.block_height) {
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(left_height), Some(right_height)) => right_height.cmp(&left_height),
            (None, None) => left.transaction_id.cmp(&right.transaction_id),
        },
    );
}

pub(crate) fn validate_transaction_hex(transaction_hex: &str) -> Result<(), String> {
    let has_valid_length = !transaction_hex.is_empty()
        && transaction_hex.len() <= MAX_TRANSACTION_HEX_LENGTH
        && transaction_hex.len().is_multiple_of(2);
    let is_hex = transaction_hex.bytes().all(|byte| byte.is_ascii_hexdigit());

    if !has_valid_length || !is_hex {
        return Err("signed transaction must be valid hexadecimal data".to_owned());
    }

    Ok(())
}

pub(crate) fn validate_transaction_id(transaction_id: &str) -> Result<(), String> {
    let is_valid =
        transaction_id.len() == 64 && transaction_id.bytes().all(|byte| byte.is_ascii_hexdigit());

    if !is_valid {
        return Err("transaction ID must be 64 hexadecimal characters".to_owned());
    }

    Ok(())
}

pub(crate) fn validate_addresses(addresses: &[String]) -> Result<(), String> {
    if addresses.len() > MAX_ADDRESSES_PER_REQUEST {
        return Err(format!(
            "wallet data requests accept at most {MAX_ADDRESSES_PER_REQUEST} addresses"
        ));
    }

    for address in addresses {
        let has_valid_length = (8..=128).contains(&address.len());
        let has_valid_characters = address
            .bytes()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit());

        if !has_valid_length || !has_valid_characters {
            return Err("wallet data request contains an invalid address".to_owned());
        }
    }

    Ok(())
}

fn address_descriptors(addresses: &[String]) -> Vec<String> {
    addresses
        .iter()
        .map(|address| format!("addr({address})"))
        .collect()
}

fn summarize_activity(
    activity: Vec<DescriptorActivity>,
    chain_height: u64,
) -> Result<Vec<ActivitySummary>, String> {
    let mut summaries_by_id = BTreeMap::<String, ActivitySummary>::new();

    for event in activity {
        match event {
            DescriptorActivity::Receive {
                amount,
                height,
                txid,
                output_spk,
            } => {
                let received_amount = north_to_atomic_units(&amount)?;
                let confirmations = confirmation_count(height, chain_height);
                let summary = summaries_by_id
                    .entry(txid.clone())
                    .or_insert_with(|| empty_activity_summary(txid, height, None, confirmations));

                summary.received_amount = summary
                    .received_amount
                    .checked_add(received_amount)
                    .ok_or_else(|| "received transaction amount is too large".to_owned())?;
                summary.received_at = output_spk.address;
                summary.confirmations = summary.confirmations.max(confirmations);
            }
            DescriptorActivity::Spend {
                amount,
                blockhash,
                height,
                spend_txid,
                ..
            } => {
                let spent_amount = north_to_atomic_units(&amount)?;
                let confirmations = confirmation_count(height, chain_height);
                let summary = summaries_by_id
                    .entry(spend_txid.clone())
                    .or_insert_with(|| {
                        empty_activity_summary(spend_txid, height, blockhash.clone(), confirmations)
                    });

                summary.spent_amount = summary
                    .spent_amount
                    .checked_add(spent_amount)
                    .ok_or_else(|| "spent transaction amount is too large".to_owned())?;
                summary.spent_input_count += 1;
                summary.block_hash = blockhash;
                summary.confirmations = summary.confirmations.max(confirmations);
            }
        }
    }

    Ok(summaries_by_id.into_values().collect())
}

fn empty_activity_summary(
    transaction_id: String,
    block_height: Option<u64>,
    block_hash: Option<String>,
    confirmations: u64,
) -> ActivitySummary {
    ActivitySummary {
        transaction_id,
        received_amount: 0,
        received_at: None,
        spent_amount: 0,
        spent_input_count: 0,
        block_height,
        block_hash,
        confirmations,
    }
}

fn received_transaction(summary: ActivitySummary) -> WalletTransaction {
    WalletTransaction {
        transaction_id: summary.transaction_id,
        received_at: summary.received_at,
        sent_to: None,
        received_amount: summary.received_amount,
        sent_amount: 0,
        fee: None,
        block_height: summary.block_height,
        confirmations: summary.confirmations,
        timestamp: None,
    }
}

fn sent_transaction(
    summary: ActivitySummary,
    raw_transaction: RawTransaction,
    wallet_addresses: &HashSet<String>,
) -> Result<WalletTransaction, String> {
    if summary.spent_input_count != raw_transaction.vin.len() {
        return Err("outgoing transaction contains inputs not owned by this wallet".to_owned());
    }

    let mut total_output_amount = 0_u64;
    let mut sent_amount = 0_u64;
    let mut sent_to = None;

    for output in raw_transaction.vout {
        let output_amount = north_to_atomic_units(&output.value)?;
        total_output_amount = total_output_amount
            .checked_add(output_amount)
            .ok_or_else(|| "outgoing transaction amount is too large".to_owned())?;

        let is_wallet_output = output
            .script_pubkey
            .address
            .as_ref()
            .is_some_and(|address| wallet_addresses.contains(address));

        if is_wallet_output {
            continue;
        }

        sent_amount = sent_amount
            .checked_add(output_amount)
            .ok_or_else(|| "outgoing payment amount is too large".to_owned())?;

        if sent_to.is_none() {
            sent_to = output.script_pubkey.address;
        }
    }

    let fee = summary
        .spent_amount
        .checked_sub(total_output_amount)
        .ok_or_else(|| "outgoing transaction outputs exceed wallet inputs".to_owned())?;

    Ok(WalletTransaction {
        transaction_id: summary.transaction_id,
        received_at: None,
        sent_to,
        received_amount: summary.received_amount,
        sent_amount,
        fee: Some(fee),
        block_height: summary.block_height,
        confirmations: summary.confirmations,
        timestamp: None,
    })
}

fn confirmation_count(block_height: Option<u64>, chain_height: u64) -> u64 {
    let Some(block_height) = block_height else {
        return 0;
    };

    chain_height
        .checked_sub(block_height)
        .and_then(|difference| difference.checked_add(1))
        .unwrap_or(0)
}

fn north_to_atomic_units(amount: &Number) -> Result<u64, String> {
    let amount = amount.to_string();
    let (mantissa, exponent) = split_decimal_exponent(&amount)?;
    let (whole_part, fractional_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));

    if whole_part.starts_with('-') {
        return Err("TrueNorth RPC returned a negative amount".to_owned());
    }

    let digits = format!("{whole_part}{fractional_part}");
    let unsigned_digits = digits.strip_prefix('+').unwrap_or(&digits);
    let unscaled_amount = unsigned_digits
        .parse::<u64>()
        .map_err(|_| "TrueNorth RPC returned an invalid amount".to_owned())?;
    let fractional_digits = i32::try_from(fractional_part.len())
        .map_err(|_| "TrueNorth RPC amount is too precise".to_owned())?;
    let atomic_scale = exponent
        .checked_sub(fractional_digits)
        .and_then(|scale| scale.checked_add(8))
        .ok_or_else(|| "TrueNorth RPC amount scale is too large".to_owned())?;

    if atomic_scale >= 0 {
        let scale = u32::try_from(atomic_scale)
            .map_err(|_| "TrueNorth RPC amount scale is too large".to_owned())?;
        let multiplier = 10_u64
            .checked_pow(scale)
            .ok_or_else(|| "TrueNorth RPC amount is too large".to_owned())?;

        return unscaled_amount
            .checked_mul(multiplier)
            .ok_or_else(|| "TrueNorth RPC amount is too large".to_owned());
    }

    let divisor_scale = atomic_scale
        .checked_abs()
        .and_then(|scale| u32::try_from(scale).ok())
        .ok_or_else(|| "TrueNorth RPC amount scale is too large".to_owned())?;
    let divisor = 10_u64
        .checked_pow(divisor_scale)
        .ok_or_else(|| "TrueNorth RPC amount is too precise".to_owned())?;

    if !unscaled_amount.is_multiple_of(divisor) {
        return Err("TrueNorth RPC amount has more than eight decimal places".to_owned());
    }

    Ok(unscaled_amount / divisor)
}

fn split_decimal_exponent(amount: &str) -> Result<(&str, i32), String> {
    let Some((mantissa, exponent)) = amount.split_once(['e', 'E']) else {
        return Ok((amount, 0));
    };

    let exponent = exponent
        .parse::<i32>()
        .map_err(|_| "TrueNorth RPC returned an invalid amount exponent".to_owned())?;

    Ok((mantissa, exponent))
}

fn fee_rate_to_atomic_units(rate_per_kilovbyte: &Number) -> Result<u64, String> {
    let atomic_units_per_kilovbyte = north_to_atomic_units(rate_per_kilovbyte)?;

    atomic_units_per_kilovbyte
        .checked_add(VBYTES_PER_KILOVBYTE - 1)
        .map(|rounded| rounded / VBYTES_PER_KILOVBYTE)
        .ok_or_else(|| "TrueNorth fee rate is too large".to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        DescriptorActivity, RawTransaction, confirmation_count, fee_rate_to_atomic_units,
        north_to_atomic_units, received_transaction, sent_transaction,
        sort_transactions_newest_first, summarize_activity, validate_transaction_hex,
        validate_transaction_id,
    };

    #[test]
    fn north_amount_is_converted_without_floating_point() {
        let amount = serde_json::from_str("10.25000001").expect("valid JSON number");

        assert_eq!(north_to_atomic_units(&amount), Ok(1_025_000_001));
    }

    #[test]
    fn scientific_notation_is_converted_without_floating_point() {
        let fee_rate = serde_json::from_str("1e-5").expect("valid JSON number");

        assert_eq!(north_to_atomic_units(&fee_rate), Ok(1_000));
        assert_eq!(fee_rate_to_atomic_units(&fee_rate), Ok(1));
    }

    #[test]
    fn fee_rate_is_rounded_up_to_atomic_units_per_vbyte() {
        let exact = serde_json::from_str("0.00001000").expect("valid JSON number");
        let rounded = serde_json::from_str("0.00001001").expect("valid JSON number");

        assert_eq!(fee_rate_to_atomic_units(&exact), Ok(1));
        assert_eq!(fee_rate_to_atomic_units(&rounded), Ok(2));
    }

    #[test]
    fn confirmation_count_includes_the_transaction_block() {
        assert_eq!(confirmation_count(Some(100), 105), 6);
        assert_eq!(confirmation_count(None, 105), 0);
    }

    #[test]
    fn mempool_activity_becomes_a_pending_transaction() {
        let activity: Vec<DescriptorActivity> = serde_json::from_str(
            r#"[{
                "type":"receive",
                "amount":1.25000000,
                "txid":"pending-transaction",
                "output_spk":{"address":"wallet-address"}
            }]"#,
        )
        .expect("mempool descriptor activity should be valid");
        let mut summaries = summarize_activity(activity, 105)
            .expect("mempool activity should summarize without a block");
        let transaction = received_transaction(summaries.remove(0));

        assert_eq!(transaction.block_height, None);
        assert_eq!(transaction.confirmations, 0);
        assert_eq!(transaction.received_amount, 125_000_000);
    }

    #[test]
    fn pending_transactions_are_shown_before_confirmed_transactions() {
        let mut transactions = vec![
            shared::WalletTransaction {
                transaction_id: "confirmed".to_owned(),
                received_at: None,
                sent_to: None,
                received_amount: 1,
                sent_amount: 0,
                fee: None,
                block_height: Some(105),
                confirmations: 1,
                timestamp: None,
            },
            shared::WalletTransaction {
                transaction_id: "pending".to_owned(),
                received_at: None,
                sent_to: None,
                received_amount: 2,
                sent_amount: 0,
                fee: None,
                block_height: None,
                confirmations: 0,
                timestamp: None,
            },
        ];

        sort_transactions_newest_first(&mut transactions);

        assert_eq!(transactions[0].transaction_id, "pending");
        assert_eq!(transactions[1].transaction_id, "confirmed");
    }

    #[test]
    fn signed_transaction_hex_is_validated_before_rpc() {
        assert_eq!(validate_transaction_hex("02000000"), Ok(()));
        assert!(validate_transaction_hex("").is_err());
        assert!(validate_transaction_hex("abc").is_err());
        assert!(validate_transaction_hex("not-hex").is_err());
    }

    #[test]
    fn transaction_id_is_validated_before_rpc() {
        let valid = "97e195eafc5824409588218a47ccf75e4377bc43555ffb30f80250c1f24124ff";

        assert_eq!(validate_transaction_id(valid), Ok(()));
        assert!(validate_transaction_id("too-short").is_err());
        assert!(validate_transaction_id(&"z".repeat(64)).is_err());
    }

    #[test]
    fn spend_and_change_become_one_sent_transaction() {
        let activity: Vec<DescriptorActivity> = serde_json::from_str(
            r#"[
                {
                    "type":"spend",
                    "amount":10.00000000,
                    "blockhash":"block-105",
                    "height":105,
                    "spend_txid":"sent-transaction"
                },
                {
                    "type":"receive",
                    "amount":8.99998360,
                    "height":105,
                    "txid":"sent-transaction",
                    "output_spk":{"address":"change-address"}
                }
            ]"#,
        )
        .expect("descriptor activity fixture should be valid");
        let mut summaries = summarize_activity(activity, 105)
            .expect("descriptor activity should summarize without floating point");
        let raw_transaction: RawTransaction = serde_json::from_str(
            r#"{
                "vin":[{}],
                "vout":[
                    {"value":1.00000000,"scriptPubKey":{"address":"recipient-address"}},
                    {"value":8.99998360,"scriptPubKey":{"address":"change-address"}}
                ]
            }"#,
        )
        .expect("raw transaction fixture should be valid");
        let wallet_addresses = HashSet::from(["change-address".to_owned()]);
        let transaction = sent_transaction(summaries.remove(0), raw_transaction, &wallet_addresses)
            .expect("outgoing transaction should be calculated");

        assert_eq!(transaction.sent_amount, 100_000_000);
        assert_eq!(transaction.received_amount, 899_998_360);
        assert_eq!(transaction.fee, Some(1_640));
        assert_eq!(transaction.sent_to.as_deref(), Some("recipient-address"));
        assert_eq!(transaction.confirmations, 1);
    }
}
