use shared::{TransactionHistory, TransactionHistoryRequest, WalletTransaction};

use super::{endpoint, http_client, require_current_index, require_success};

pub async fn fetch_transaction_history(
    addresses: Vec<String>,
) -> Result<Vec<WalletTransaction>, String> {
    let client = http_client()?;
    // A new block or pending snapshot invalidates the cursor. Discard partial
    // pages and retry once so histories from different snapshots are not mixed.
    for _ in 0..2 {
        let mut transactions = Vec::new();
        let mut cursor = None;
        for _ in 0..100 {
            let request = TransactionHistoryRequest {
                addresses: addresses.clone(),
                cursor,
                limit: Some(100),
            };
            let response = client
                .post(endpoint("/v1/transactions")?)
                .json(&request)
                .send()
                .await
                .map_err(|error| format!("could not contact wallet server: {error}"))?;
            if response.status() == reqwest::StatusCode::CONFLICT {
                break;
            }
            let response = require_success(response, "Could not load transaction history").await?;
            let page: TransactionHistory = response
                .json()
                .await
                .map_err(|error| format!("wallet server returned invalid history: {error}"))?;
            require_current_index(&page.index)?;
            if page.transactions.len() > 100
                || page
                    .next_cursor
                    .as_ref()
                    .is_some_and(|cursor| cursor.len() > 1024)
            {
                return Err("The wallet server returned an oversized history page.".into());
            }
            transactions.extend(page.transactions);
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(transactions);
            }
            if transactions.len() >= 10_000 {
                return Err(
                    "Transaction history exceeds the current 10,000 transaction display limit."
                        .into(),
                );
            }
        }
    }
    Err("Transaction history changed while loading. Please refresh.".into())
}
