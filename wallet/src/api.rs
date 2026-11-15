mod addresses;
mod history;

pub use addresses::{fetch_used_addresses, fetch_utxos};
pub use history::fetch_transaction_history;

use std::time::Duration;

use shared::{
    ApiErrorResponse, BroadcastTransactionResponse, ChainTip, FeeEstimateV2, IndexStatus,
    SignedTransactionRequest, TransactionStatusRequest, TransactionStatusResponse,
};

const WALLET_SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(WALLET_SERVER_REQUEST_TIMEOUT)
        // Prevent redirects from bypassing the configured server's transport policy.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("could not create wallet server client: {error}"))
}

fn endpoint(path: &str) -> Result<String, String> {
    let base_url = crate::config::current_api_base_url()?;
    Ok(format!("{base_url}{path}"))
}

pub async fn fetch_chain_height() -> Result<u64, String> {
    let response = http_client()?
        .get(endpoint("/v1/chain/tip")?)
        .send()
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

pub async fn fetch_fee_estimate() -> Result<FeeEstimateV2, String> {
    let response = http_client()?
        .get(endpoint("/v2/fees/estimate")?)
        .send()
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("wallet server could not estimate the network fee: {error}"))?;

    response
        .json::<FeeEstimateV2>()
        .await
        .map_err(|error| format!("wallet server returned an invalid fee estimate: {error}"))
}

pub async fn broadcast_signed_transaction(transaction_hex: String) -> Result<String, String> {
    let request = SignedTransactionRequest { transaction_hex };
    let response = http_client()?
        .post(endpoint("/v1/transactions/broadcast")?)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("could not contact wallet server: {error}"))?;

    let response = require_success(response, "Could not send the transaction").await?;

    let result = response
        .json::<BroadcastTransactionResponse>()
        .await
        .map_err(|error| format!("wallet server returned an invalid broadcast result: {error}"))?;

    Ok(result.transaction_id)
}

pub async fn fetch_transaction_status(transaction_id: String) -> Result<bool, String> {
    let request = TransactionStatusRequest { transaction_id };
    let response = http_client()?
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

async fn require_success(
    response: reqwest::Response,
    context: &str,
) -> Result<reqwest::Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    match response.json::<ApiErrorResponse>().await {
        Ok(error) => Err(format!("{context}: {}", error.message)),
        Err(_) => Err(format!("{context}: server returned {status}")),
    }
}

fn require_current_index(status: &IndexStatus) -> Result<(), String> {
    if !status.ready
        || !status.pending_available
        || status.indexed_height.is_none()
        || status.indexed_height != status.observed_node_height
        || status.indexed_hash.is_none()
        || status.indexed_hash != status.observed_node_hash
    {
        return Err("The wallet server is synchronizing or stale. Retry shortly.".into());
    }
    Ok(())
}
