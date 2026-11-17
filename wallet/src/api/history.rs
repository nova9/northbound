use std::collections::{HashMap, HashSet};

use shared::{IndexStatus, TransactionHistory, TransactionHistoryRequest, WalletTransaction};

use crate::config::MAX_ADDRESSES_PER_REQUEST;

use super::addresses::unique_addresses;
use super::{endpoint, http_client, require_current_index, require_success};

const HISTORY_PAGE_SIZE: u16 = 100;
const MAX_HISTORY_CURSOR_LENGTH: usize = 1_024;

pub async fn fetch_transaction_history(
    addresses: Vec<String>,
) -> Result<Vec<WalletTransaction>, String> {
    if addresses.is_empty() {
        return Ok(Vec::new());
    }
    let url = endpoint("/v1/transactions")?;
    fetch_transaction_history_from_server(addresses, &url).await
}

async fn fetch_transaction_history_from_server(
    addresses: Vec<String>,
    url: &str,
) -> Result<Vec<WalletTransaction>, String> {
    let addresses = unique_addresses(addresses);
    let client = http_client()?;
    // Retry the whole load once, including completed address batches, if the
    // chain or pending snapshot changes. Never return a partial history.
    'attempt: for _ in 0..2 {
        let mut transactions = HashMap::new();
        let mut snapshot: Option<IndexStatus> = None;
        for batch in addresses.chunks(MAX_ADDRESSES_PER_REQUEST) {
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
                let response = client
                    .post(url)
                    .json(&request)
                    .send()
                    .await
                    .map_err(|error| format!("could not contact wallet server: {error}"))?;
                if response.status() == reqwest::StatusCode::CONFLICT {
                    continue 'attempt;
                }
                let response =
                    require_success(response, "Could not load transaction history").await?;
                let page: TransactionHistory = response
                    .json()
                    .await
                    .map_err(|error| format!("wallet server returned invalid history: {error}"))?;
                require_current_index(&page.index)?;
                if page.index.pending_revision.is_none() {
                    return Err(
                        "The wallet server returned history without a snapshot revision.".into(),
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
                for transaction in page.transactions {
                    // Repetition across address batches is expected. Within a
                    // batch's pages it would count the same contribution twice.
                    if !seen_transactions.insert(transaction.transaction_id.clone()) {
                        return Err(
                            "The wallet server repeated a transaction within a history batch."
                                .into(),
                        );
                    }
                    let height = transaction.block_height.unwrap_or(u64::MAX);
                    if let Some((previous_height, previous_id)) = &previous_position {
                        if height > *previous_height
                            || (height == *previous_height
                                && transaction.transaction_id <= *previous_id)
                        {
                            return Err("The wallet server returned history out of order.".into());
                        }
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::api::test_support::{addresses, index, serve};

    fn transaction(number: usize, net_amount: i64) -> Value {
        json!({
            "transaction_id": format!("{number:064x}"),
            "net_amount": net_amount, "network_fee": 10_000_000,
            "block_height": 120, "confirmations": 1, "timestamp": 1000
        })
    }

    #[test]
    fn address_batches_must_agree_on_whether_a_transaction_is_a_mining_reward() {
        let mut transactions = HashMap::new();
        let mut first: WalletTransaction = serde_json::from_value(transaction(1, 100)).unwrap();
        first.coinbase = true;
        merge_transaction(&mut transactions, first.clone()).unwrap();
        merge_transaction(&mut transactions, first.clone()).unwrap();
        assert!(transactions[&first.transaction_id].coinbase);
        first.coinbase = false;
        assert!(merge_transaction(&mut transactions, first).is_err());
    }

    #[test]
    fn immature_rewards_remain_in_history_with_current_and_older_servers() {
        for includes_coinbase_field in [false, true] {
            let (url, server) = serve(1, move |_, _| {
                let mut reward = transaction(1, 51_200_000_000);
                reward["network_fee"] = Value::Null;
                reward["confirmations"] = json!(68);
                reward["block_height"] = json!(53);
                if includes_coinbase_field {
                    reward["coinbase"] = json!(true);
                }
                (200, page(vec![transaction(2, -100), reward], None))
            });
            let history = fetch(addresses(1), &url).unwrap();
            server.join().unwrap();
            assert_eq!(history.len(), 2);
            let reward = history
                .iter()
                .find(|transaction| transaction.net_amount == 51_200_000_000)
                .unwrap();
            assert_eq!(reward.confirmations, 68);
            assert_eq!(reward.coinbase, includes_coinbase_field);
        }
    }

    fn page(transactions: Vec<Value>, cursor: Option<&str>) -> Value {
        json!({"transactions": transactions, "next_cursor": cursor, "index": index()})
    }

    fn fetch(addresses: Vec<String>, url: &str) -> Result<Vec<WalletTransaction>, String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fetch_transaction_history_from_server(addresses, url))
    }

    #[test]
    fn merges_every_address_batch_and_page_into_one_sorted_item_per_transaction() {
        let (url, server) = serve(4, |number, request| {
            assert_eq!(request["limit"], 100);
            let body = match number {
                0 => {
                    let mut pending = transaction(0, 0);
                    pending["block_height"] = Value::Null;
                    pending["confirmations"] = json!(0);
                    page(vec![pending, transaction(1, 300_000_000)], Some("page-2"))
                }
                1 => {
                    assert_eq!(request["cursor"], "page-2");
                    page(vec![transaction(2, 100_000_000)], None)
                }
                2 => {
                    assert!(request["cursor"].is_null());
                    page(vec![transaction(1, -800_000_000)], None)
                }
                3 => page(vec![transaction(2, 200_000_000)], None),
                _ => unreachable!(),
            };
            (200, body)
        });
        let mut requested = addresses(201);
        requested.extend(addresses(101));
        let history = fetch(requested, &url).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].transaction_id, format!("{:064x}", 0));
        assert_eq!(history[0].net_amount, 0);
        assert_eq!(history[1].transaction_id, format!("{:064x}", 1));
        assert_eq!(history[1].net_amount, -500_000_000);
        assert_eq!(history[1].network_fee, Some(10_000_000));
        assert_eq!(history[2].net_amount, 300_000_000);
        let requests = server.join().unwrap();
        assert_eq!(requests[0]["addresses"], requests[1]["addresses"]);
        let mut unique_requested = Vec::new();
        for request in [&requests[0], &requests[2], &requests[3]] {
            let batch = request["addresses"].as_array().unwrap();
            assert!(batch.len() <= 100);
            unique_requested.extend(
                batch
                    .iter()
                    .map(|address| address.as_str().unwrap().to_owned()),
            );
        }
        assert_eq!(unique_requested, addresses(201));
    }

    #[test]
    fn change_in_another_batch_merges_without_counting_the_fee_again() {
        let (url, server) = serve(2, |number, _| {
            let net = if number == 0 {
                -1_000_000_000
            } else {
                690_000_000
            };
            (200, page(vec![transaction(1, net)], None))
        });
        let history = fetch(addresses(101), &url).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].net_amount, -310_000_000);
        assert_eq!(history[0].network_fee, Some(10_000_000));
        server.join().unwrap();
    }

    #[test]
    fn loads_more_than_ten_thousand_transactions_without_a_total_cap() {
        let total = 10_005_usize;
        let (url, server) = serve(total.div_ceil(100), move |number, request| {
            if number == 0 {
                assert!(request["cursor"].is_null());
            } else {
                assert_eq!(request["cursor"], format!("page-{number}"));
            }
            let mut transactions = Vec::new();
            for id in number * 100..((number + 1) * 100).min(total) {
                transactions.push(transaction(id, 1));
            }
            let next = if (number + 1) * 100 < total {
                Some(format!("page-{}", number + 1))
            } else {
                None
            };
            (200, page(transactions, next.as_deref()))
        });
        let history = fetch(addresses(1), &url).unwrap();
        assert_eq!(history.len(), total);
        assert_eq!(
            history.last().unwrap().transaction_id,
            format!("{:064x}", total - 1)
        );
        server.join().unwrap();
    }

    #[test]
    fn restarts_all_batches_after_cursor_or_snapshot_changes() {
        for change in ["conflict", "revision", "tip"] {
            let (url, server) = serve(4, move |number, _| {
                if number == 1 && change == "conflict" {
                    return (409, json!({"message": "Snapshot changed"}));
                }
                let mut body = page(
                    vec![transaction(1, if number < 2 { 100 } else { 200 })],
                    None,
                );
                if number >= 1 && change == "revision" {
                    body["index"]["pending_revision"] = json!(8);
                }
                if number >= 1 && change == "tip" {
                    body["index"]["indexed_hash"] = json!("cd".repeat(32));
                    body["index"]["observed_node_hash"] = json!("cd".repeat(32));
                }
                (200, body)
            });
            let history = fetch(addresses(101), &url).unwrap();
            assert_eq!(history[0].net_amount, 400);
            let requests = server.join().unwrap();
            assert_eq!(requests[0]["addresses"], requests[2]["addresses"]);
            assert_eq!(requests[1]["addresses"], requests[3]["addresses"]);
        }
    }

    #[test]
    fn rejects_partial_inconsistent_or_repeated_history() {
        for invalid in [
            "duplicate",
            "overflow",
            "fee",
            "metadata",
            "error",
            "missing-net",
            "oversized",
            "empty",
            "cursor-cycle",
            "cursor-length",
            "order",
            "stale",
            "revision-missing",
        ] {
            let within_batch = matches!(invalid, "duplicate" | "cursor-cycle" | "order");
            let (url, server) = serve(2, move |number, _| {
                if number == 0 {
                    return (
                        200,
                        page(
                            vec![transaction(
                                1,
                                if invalid == "overflow" { i64::MAX } else { 100 },
                            )],
                            if within_batch {
                                Some("next-page")
                            } else {
                                None
                            },
                        ),
                    );
                }
                let mut body = page(vec![transaction(if within_batch { 2 } else { 1 }, 1)], None);
                match invalid {
                    "duplicate" => body = page(vec![transaction(1, 1)], None),
                    "overflow" => {}
                    "fee" => body["transactions"][0]["network_fee"] = json!(5),
                    "metadata" => body["transactions"][0]["timestamp"] = json!(999),
                    "error" => return (503, json!({"message": "Unavailable"})),
                    "missing-net" => {
                        body["transactions"][0]
                            .as_object_mut()
                            .unwrap()
                            .remove("net_amount");
                    }
                    "oversized" => body["transactions"] = json!(vec![transaction(2, 0); 101]),
                    "empty" => body = page(vec![], Some("page-3")),
                    "cursor-cycle" => body["next_cursor"] = json!("next-page"),
                    "cursor-length" => {
                        body["next_cursor"] = json!("a".repeat(MAX_HISTORY_CURSOR_LENGTH + 1))
                    }
                    "order" => body = page(vec![transaction(0, 0)], None),
                    "stale" => body["index"]["ready"] = json!(false),
                    "revision-missing" => body["index"]["pending_revision"] = Value::Null,
                    _ => unreachable!(),
                }
                (200, body)
            });
            assert!(fetch(addresses(101), &url).is_err(), "{invalid}");
            server.join().unwrap();
        }
    }

    #[test]
    fn bounds_snapshot_retries_and_empty_addresses_need_no_server() {
        let (url, server) = serve(2, |_, _| (409, json!({"message": "Changed"})));
        assert!(fetch(addresses(101), &url).unwrap_err().contains("changed"));
        server.join().unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                assert!(
                    fetch_transaction_history(Vec::new())
                        .await
                        .unwrap()
                        .is_empty()
                );
            });
    }
}
