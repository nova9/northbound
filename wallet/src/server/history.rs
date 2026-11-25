use std::collections::{HashMap, HashSet};

use shared::{IndexStatus, TransactionHistory, TransactionHistoryRequest, WalletTransaction};

use crate::config::MAX_ADDRESSES_PER_REQUEST;
use crate::server::addresses::unique_addresses;
use crate::server::client::{ServerClient, require_current_index, require_success};

const HISTORY_PAGE_SIZE: u16 = 100;
const MAX_HISTORY_CURSOR_LENGTH: usize = 1_024;

impl ServerClient {
    pub(crate) async fn fetch_transaction_history(
        &self,
        addresses: Vec<String>,
    ) -> Result<Vec<WalletTransaction>, String> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let addresses = unique_addresses(addresses);
        let url = self.endpoint("/v1/transactions");
        // Retry the whole load once, including completed address batches, if the
        // chain or pending snapshot changes. Never return a partial history.
        'attempt: for _ in 0..2 {
            let mut transactions = HashMap::new();
            let mut snapshot: Option<IndexStatus> = None;
            for batch in addresses.chunks(MAX_ADDRESSES_PER_REQUEST) {
                let requested_in_batch = batch.iter().map(String::as_str).collect::<HashSet<_>>();
                let mut cursor = None;
                let mut seen_cursors = HashSet::new();
                let mut seen_transactions = HashSet::new();
                let mut previous_position: Option<(u64, String)> = None;
                loop {
                    let request = TransactionHistoryRequest {
                        addresses: batch.to_vec(),
                        cursor,
                        limit: Some(HISTORY_PAGE_SIZE),
                    };
                    let response = self
                        .http()
                        .post(&url)
                        .json(&request)
                        .send()
                        .await
                        .map_err(|error| format!("could not contact wallet server: {error}"))?;
                    if response.status() == reqwest::StatusCode::CONFLICT {
                        continue 'attempt;
                    }
                    let response =
                        require_success(response, "Could not load transaction history").await?;
                    let page: TransactionHistory = response.json().await.map_err(|error| {
                        format!("wallet server returned invalid history: {error}")
                    })?;
                    require_current_index(&page.index)?;
                    if page.index.pending_revision.is_none() {
                        return Err(
                            "The wallet server returned history without a snapshot revision."
                                .into(),
                        );
                    }
                    if let Some(first) = &snapshot {
                        if first.indexed_height != page.index.indexed_height
                            || first.indexed_hash != page.index.indexed_hash
                            || first.pending_revision != page.index.pending_revision
                        {
                            continue 'attempt;
                        }
                    } else {
                        snapshot = Some(page.index);
                    }
                    if page.transactions.len() > usize::from(HISTORY_PAGE_SIZE)
                        || page
                            .next_cursor
                            .as_ref()
                            .is_some_and(|cursor| cursor.len() > MAX_HISTORY_CURSOR_LENGTH)
                        || (page.transactions.is_empty() && page.next_cursor.is_some())
                    {
                        return Err("The wallet server returned an invalid history page.".into());
                    }
                    for mut transaction in page.transactions {
                        // Repetition across address batches is expected. Within a
                        // batch's pages it would count the same contribution twice.
                        if !seen_transactions.insert(transaction.transaction_id.clone()) {
                            return Err(
                                "The wallet server repeated a transaction within a history batch."
                                    .into(),
                            );
                        }
                        for address in &transaction.received_addresses {
                            if !requested_in_batch.contains(address.as_str()) {
                                return Err(
                                    "The wallet server returned an unrequested address in transaction history."
                                        .into(),
                                );
                            }
                        }
                        normalize_addresses(&mut transaction.received_addresses);
                        let height = transaction.block_height.unwrap_or(u64::MAX);
                        if let Some((previous_height, previous_id)) = &previous_position
                            && (height > *previous_height
                                || (height == *previous_height
                                    && transaction.transaction_id <= *previous_id))
                        {
                            return Err("The wallet server returned history out of order.".into());
                        }
                        previous_position = Some((height, transaction.transaction_id.clone()));
                        merge_transaction(&mut transactions, transaction)?;
                    }
                    cursor = page.next_cursor;
                    let Some(next_cursor) = &cursor else {
                        break;
                    };
                    if !seen_cursors.insert(next_cursor.clone()) {
                        return Err("The wallet server repeated a history cursor.".into());
                    }
                }
            }
            let mut transactions: Vec<_> = transactions.into_values().collect();
            transactions.sort_by(|left, right| {
                right
                    .block_height
                    .unwrap_or(u64::MAX)
                    .cmp(&left.block_height.unwrap_or(u64::MAX))
                    .then_with(|| left.transaction_id.cmp(&right.transaction_id))
            });
            return Ok(transactions);
        }
        Err("Transaction history changed while loading. Please refresh.".into())
    }
}

fn merge_transaction(
    transactions: &mut HashMap<String, WalletTransaction>,
    incoming: WalletTransaction,
) -> Result<(), String> {
    let Some(existing) = transactions.get_mut(&incoming.transaction_id) else {
        transactions.insert(incoming.transaction_id.clone(), incoming);
        return Ok(());
    };
    if existing.block_height != incoming.block_height
        || existing.confirmations != incoming.confirmations
        || existing.timestamp != incoming.timestamp
        || existing.network_fee != incoming.network_fee
        || existing.coinbase != incoming.coinbase
    {
        return Err("The wallet server returned conflicting transaction details.".into());
    }
    // Fee effects are already in each contribution. Keep the transaction-wide
    // fee once for display; never sum it or subtract it from the net amount.
    existing.net_amount = existing
        .net_amount
        .checked_add(incoming.net_amount)
        .ok_or_else(|| "Combined history net amount overflow.".to_owned())?;
    existing
        .received_addresses
        .extend(incoming.received_addresses);
    normalize_addresses(&mut existing.received_addresses);
    Ok(())
}

fn normalize_addresses(addresses: &mut Vec<String>) {
    addresses.sort();
    addresses.dedup();
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
