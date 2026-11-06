use shared::{
    AddressActivityRequest, AddressActivityResponse, BroadcastTransactionResponse, ChainTip,
    FeeEstimate, SignedTransactionRequest, TransactionHistory, TransactionHistoryRequest,
    TransactionStatusRequest, TransactionStatusResponse, TransactionValidation, UtxoRequest,
    UtxoResponse, WalletTransaction, WalletUtxo,
};

fn endpoint(path: &str) -> Result<String, String> {
    let base_url = crate::config::current_api_base_url()?;
    Ok(format!("{base_url}{path}"))
}

pub async fn fetch_chain_height() -> Result<u64, String> {
    let response = reqwest::get(endpoint("/v1/chain/tip")?)
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("wallet server returned an error: {error}"))?;

    let chain_tip = response
        .json::<ChainTip>()
        .await
        .map_err(|error| format!("wallet server returned invalid JSON: {error}"))?;

    Ok(chain_tip.height)
}

pub async fn fetch_fee_estimate() -> Result<FeeEstimate, String> {
    let response = reqwest::get(endpoint("/v1/fees/estimate")?)
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("wallet server could not estimate the network fee: {error}"))?;

    response
        .json::<FeeEstimate>()
        .await
        .map_err(|error| format!("wallet server returned an invalid fee estimate: {error}"))
}

pub async fn fetch_used_addresses(addresses: Vec<String>) -> Result<Vec<String>, String> {
    let request = AddressActivityRequest { addresses };
    let response = reqwest::Client::new()
        .post(endpoint("/v1/addresses/activity")?)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("wallet server could not scan address activity: {error}"))?;

    let response = response
        .json::<AddressActivityResponse>()
        .await
        .map_err(|error| format!("wallet server returned invalid address activity: {error}"))?;

    Ok(response.used_addresses)
}

pub async fn fetch_transaction_history(
    addresses: Vec<String>,
) -> Result<Vec<WalletTransaction>, String> {
    let request = TransactionHistoryRequest { addresses };
    let response = reqwest::Client::new()
        .post(endpoint("/v1/transactions")?)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("wallet server could not load transaction history: {error}"))?;

    let history = response
        .json::<TransactionHistory>()
        .await
        .map_err(|error| format!("wallet server returned invalid transaction history: {error}"))?;

    Ok(history.transactions)
}

pub async fn fetch_utxos(addresses: Vec<String>) -> Result<Vec<WalletUtxo>, String> {
    let request = UtxoRequest { addresses };
    let response = reqwest::Client::new()
        .post(endpoint("/v1/utxos")?)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("wallet server could not load spendable outputs: {error}"))?;

    let response = response
        .json::<UtxoResponse>()
        .await
        .map_err(|error| format!("wallet server returned invalid spendable outputs: {error}"))?;

    Ok(response.utxos)
}

pub async fn validate_signed_transaction(
    transaction_hex: String,
) -> Result<TransactionValidation, String> {
    let request = SignedTransactionRequest { transaction_hex };
    let response = reqwest::Client::new()
        .post(endpoint("/v1/transactions/validate")?)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("wallet server could not validate the transaction: {error}"))?;

    response
        .json::<TransactionValidation>()
        .await
        .map_err(|error| format!("wallet server returned an invalid validation result: {error}"))
}

pub async fn broadcast_signed_transaction(transaction_hex: String) -> Result<String, String> {
    let request = SignedTransactionRequest { transaction_hex };
    let response = reqwest::Client::new()
        .post(endpoint("/v1/transactions/broadcast")?)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("wallet server could not broadcast the transaction: {error}"))?;

    let result = response
        .json::<BroadcastTransactionResponse>()
        .await
        .map_err(|error| format!("wallet server returned an invalid broadcast result: {error}"))?;

    Ok(result.transaction_id)
}

pub async fn fetch_transaction_status(transaction_id: String) -> Result<bool, String> {
    let request = TransactionStatusRequest { transaction_id };
    let response = reqwest::Client::new()
        .post(endpoint("/v1/transactions/status")?)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;
    let response = response
        .error_for_status()
        .map_err(|error| format!("wallet server could not check transaction status: {error}"))?;
    let result = response
        .json::<TransactionStatusResponse>()
        .await
        .map_err(|error| {
            format!("wallet server returned an invalid transaction status: {error}")
        })?;

    Ok(result.known)
}
