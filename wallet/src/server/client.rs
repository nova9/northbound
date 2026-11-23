use std::time::Duration;

use shared::{
    ApiErrorResponse, BroadcastTransactionResponse, ChainTip, FeeEstimateV2, IndexStatus,
    ServerInfo, SignedTransactionRequest, TransactionStatusRequest, TransactionStatusResponse,
};

use crate::config::WALLET_NETWORK;
use crate::storage::settings;

const WALLET_SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct ServerClient {
    http: reqwest::Client,
    base_url: String,
}

impl ServerClient {
    pub(crate) fn from_configured_server() -> Result<Self, String> {
        let base_url = settings::current_api_base_url()?;
        Self::new(base_url)
    }

    pub(crate) fn new(base_url: impl Into<String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(WALLET_SERVER_REQUEST_TIMEOUT)
            // Prevent redirects from bypassing the configured server's transport policy.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| format!("could not create wallet server client: {error}"))?;

        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    pub(super) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub(super) fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    pub(crate) async fn check_compatibility(&self) -> Result<(), String> {
        let response = self
            .http
            .get(self.endpoint("/info"))
            .send()
            .await
            .map_err(|_| "Could not reach the server to check compatibility.".to_owned())?;
        if !response.status().is_success() {
            return Err(format!(
                "The server returned HTTP {} for /info.",
                response.status()
            ));
        }
        let info = response.json::<ServerInfo>().await.map_err(|_| {
            "The server did not provide valid version and network information.".to_owned()
        })?;
        let expected_network = WALLET_NETWORK.server_name();
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

    pub(crate) async fn fetch_chain_height(&self) -> Result<u64, String> {
        let response = self
            .http
            .get(self.endpoint("/v1/chain/tip"))
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

    pub(crate) async fn fetch_fee_estimate(&self) -> Result<FeeEstimateV2, String> {
        let response = self
            .http
            .get(self.endpoint("/v2/fees/estimate"))
            .send()
            .await
            .map_err(|error| format!("could not contact wallet server: {error}"))?;

        let response = response.error_for_status().map_err(|error| {
            format!("wallet server could not estimate the network fee: {error}")
        })?;

        response
            .json::<FeeEstimateV2>()
            .await
            .map_err(|error| format!("wallet server returned an invalid fee estimate: {error}"))
    }

    pub(crate) async fn broadcast_signed_transaction(
        &self,
        transaction_hex: String,
    ) -> Result<String, String> {
        let request = SignedTransactionRequest { transaction_hex };
        let response = self
            .http
            .post(self.endpoint("/v1/transactions/broadcast"))
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("could not contact wallet server: {error}"))?;

        let response = require_success(response, "Could not send the transaction").await?;

        let result = response
            .json::<BroadcastTransactionResponse>()
            .await
            .map_err(|error| {
                format!("wallet server returned an invalid broadcast result: {error}")
            })?;

        Ok(result.transaction_id)
    }

    pub(crate) async fn fetch_transaction_status(
        &self,
        transaction_id: String,
    ) -> Result<bool, String> {
        let request = TransactionStatusRequest { transaction_id };
        let response = self
            .http
            .post(self.endpoint("/v1/transactions/status"))
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("could not contact wallet server: {error}"))?;
        let response = response.error_for_status().map_err(|error| {
            format!("wallet server could not check transaction status: {error}")
        })?;
        let result = response
            .json::<TransactionStatusResponse>()
            .await
            .map_err(|error| {
                format!("wallet server returned an invalid transaction status: {error}")
            })?;

        Ok(result.known)
    }
}

pub(super) async fn require_success(
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

pub(super) fn require_current_index(status: &IndexStatus) -> Result<(), String> {
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
    use serde_json::json;

    use super::{ServerClient, require_success};
    use crate::config::WALLET_NETWORK;
    use crate::server::test_support::serve;

    #[tokio::test]
    async fn checks_network_and_release_at_the_candidate_server() {
        let network = WALLET_NETWORK.server_name();
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
            let client = ServerClient::new(&url).unwrap();
            assert_eq!(client.check_compatibility().await.is_ok(), compatible);
            assert_eq!(server.join().unwrap(), [json!("GET /info HTTP/1.1")]);
        }
    }

    #[tokio::test]
    async fn broadcast_rejection_reasons_remain_visible_to_the_wallet() {
        for status in [400, 409, 502] {
            let (url, server) = serve(1, move |_, _| (status, json!({"message": "node reason"})));
            let client = ServerClient::new(&url).unwrap();
            let response = client
                .http()
                .post(&url)
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
