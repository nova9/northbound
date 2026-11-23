mod addresses;
mod history;

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use addresses::fetch_used_addresses_from_server;
pub use addresses::fetch_utxos;
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

pub(crate) fn address_activity_endpoint() -> Result<String, String> {
    endpoint("/v1/addresses/activity")
}

pub(crate) async fn check_server_compatibility(base_url: &str) -> Result<(), String> {
    let response = http_client()?
        .get(format!("{base_url}/info"))
        .send()
        .await
        .map_err(|_| "Could not reach the server to check compatibility.".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "The server returned HTTP {} for /info.",
            response.status()
        ));
    }
    let info = response.json::<shared::ServerInfo>().await.map_err(|_| {
        "The server did not provide valid version and network information.".to_owned()
    })?;
    let expected_network = match crate::config::WALLET_NETWORK {
        crate::p2qrh::Network::Mainnet => "main",
        crate::p2qrh::Network::Regtest => "regtest",
    };
    if info.network != expected_network {
        return Err(format!(
            "Server network '{}' differs from the expected '{expected_network}'.",
            info.network
        ));
    }
    let wallet_version = env!("CARGO_PKG_VERSION");
    if info.version != wallet_version {
        return Err(format!(
            "Server version '{}' differs from wallet version '{wallet_version}'. Compatibility is unconfirmed.",
            info.version
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::serve;
    use serde_json::json;

    #[tokio::test]
    async fn checks_network_and_release_at_the_candidate_server() {
        let network = if cfg!(feature = "mainnet") {
            "main"
        } else {
            "regtest"
        };
        let version = env!("CARGO_PKG_VERSION");
        for (status, body, compatible) in [
            (200, json!({"network": network, "version": version}), true),
            (
                200,
                json!({"network": "unknown", "version": version}),
                false,
            ),
            (200, json!({"network": network, "version": "0.1.0"}), false),
            (200, json!({"version": version}), false),
            (404, json!({}), false),
        ] {
            let (url, server) = serve(1, move |_, _| (status, body.clone()));
            assert_eq!(check_server_compatibility(&url).await.is_ok(), compatible);
            assert_eq!(server.join().unwrap(), [json!("GET /info HTTP/1.1")]);
        }
    }

    #[tokio::test]
    async fn broadcast_rejection_reasons_remain_visible_to_the_wallet() {
        for status in [400, 409, 502] {
            let (url, server) = serve(1, move |_, _| (status, json!({"message": "node reason"})));
            let response = http_client()
                .unwrap()
                .post(url)
                .json(&json!({}))
                .send()
                .await
                .unwrap();
            let error = require_success(response, "Could not send the transaction")
                .await
                .unwrap_err();
            assert_eq!(error, "Could not send the transaction: node reason");
            server.join().unwrap();
        }
    }
}
