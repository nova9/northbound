use std::collections::HashSet;

use shared::{
    AddressActivityRequest, AddressActivityResponse, DEFAULT_UTXO_PAGE_SIZE, IndexStatus,
    MAX_UTXO_CURSOR_LENGTH, UtxoRequest, UtxoResponse, WalletUtxo,
};

use crate::config::MAX_ADDRESSES_PER_REQUEST;

use super::{endpoint, http_client, require_current_index, require_success};

pub async fn fetch_used_addresses(addresses: Vec<String>) -> Result<Vec<String>, String> {
    if addresses.is_empty() {
        return Ok(Vec::new());
    }
    let url = endpoint("/v1/addresses/activity")?;
    fetch_used_addresses_from_server(addresses, &url).await
}

async fn fetch_used_addresses_from_server(
    addresses: Vec<String>,
    url: &str,
) -> Result<Vec<String>, String> {
    let addresses = unique_addresses(addresses);
    let client = http_client()?;
    let mut snapshot = None;
    let mut used_addresses = Vec::new();
    let mut seen_addresses = HashSet::new();

    for batch in addresses.chunks(MAX_ADDRESSES_PER_REQUEST) {
        let request = AddressActivityRequest {
            addresses: batch.to_vec(),
        };
        let response = client
            .post(url)
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("could not contact wallet server: {error}"))?;
        let response = require_success(response, "Could not load address activity").await?;
        let response = response
            .json::<AddressActivityResponse>()
            .await
            .map_err(|error| format!("wallet server returned invalid address activity: {error}"))?;
        require_matching_snapshot(&mut snapshot, &response.index, addresses.len())?;
        for address in response.used_addresses {
            if !batch.contains(&address) {
                return Err(
                    "The wallet server returned activity for an unrequested address.".into(),
                );
            }
            if seen_addresses.insert(address.clone()) {
                used_addresses.push(address);
            }
        }
    }
    Ok(used_addresses)
}

pub async fn fetch_utxos(addresses: Vec<String>) -> Result<Vec<WalletUtxo>, String> {
    if addresses.is_empty() {
        return Ok(Vec::new());
    }
    let url = endpoint("/v1/utxos")?;
    fetch_utxos_from_server(addresses, &url).await
}

async fn fetch_utxos_from_server(
    addresses: Vec<String>,
    url: &str,
) -> Result<Vec<WalletUtxo>, String> {
    let addresses = unique_addresses(addresses);
    let client = http_client()?;
    let mut snapshot = None;
    let mut utxos = Vec::new();
    let mut seen_outpoints = HashSet::new();

    for batch in addresses.chunks(MAX_ADDRESSES_PER_REQUEST) {
        let mut cursor = None;
        let mut seen_cursors = HashSet::new();
        let mut previous_output = None;
        loop {
            let request = UtxoRequest {
                addresses: batch.to_vec(),
                cursor,
                limit: Some(DEFAULT_UTXO_PAGE_SIZE),
            };
            let response = client
                .post(url)
                .json(&request)
                .send()
                .await
                .map_err(|error| format!("could not contact wallet server: {error}"))?;
            if response.status() == reqwest::StatusCode::CONFLICT {
                return Err("Spendable outputs changed while loading. Please refresh.".into());
            }
            let response = require_success(response, "Could not load spendable outputs").await?;
            let response = response.json::<UtxoResponse>().await.map_err(|error| {
                format!("wallet server returned invalid spendable outputs: {error}")
            })?;
            require_matching_snapshot(&mut snapshot, &response.index, addresses.len())?;
            if (request.cursor.is_some() || response.next_cursor.is_some())
                && response.index.pending_revision.is_none()
            {
                return Err(
                    "The wallet server must be updated to support paginated outputs.".into(),
                );
            }
            if response.utxos.len() > usize::from(DEFAULT_UTXO_PAGE_SIZE)
                || response
                    .next_cursor
                    .as_ref()
                    .is_some_and(|cursor| cursor.len() > MAX_UTXO_CURSOR_LENGTH)
                || (response.utxos.is_empty() && response.next_cursor.is_some())
            {
                return Err("The wallet server returned an invalid spendable output page.".into());
            }
            for utxo in response.utxos {
                let outpoint = (utxo.transaction_id.clone(), utxo.output_index);
                if !seen_outpoints.insert(outpoint) {
                    return Err("The wallet server returned a duplicate spendable output.".into());
                }
                let position = (
                    utxo.block_height,
                    utxo.transaction_id.clone(),
                    utxo.output_index,
                );
                if previous_output
                    .as_ref()
                    .is_some_and(|previous| previous >= &position)
                {
                    return Err("The wallet server returned spendable outputs out of order.".into());
                }
                previous_output = Some(position);
                utxos.push(utxo);
            }
            cursor = response.next_cursor;
            let Some(next_cursor) = &cursor else {
                break;
            };
            if !seen_cursors.insert(next_cursor.clone()) {
                return Err("The wallet server repeated a spendable output cursor.".into());
            }
        }
    }
    Ok(utxos)
}

pub(super) fn unique_addresses(addresses: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for address in addresses {
        if seen.insert(address.clone()) {
            unique.push(address);
        }
    }
    unique
}

fn require_matching_snapshot(
    first: &mut Option<IndexStatus>,
    current: &IndexStatus,
    address_count: usize,
) -> Result<(), String> {
    require_current_index(current)?;
    if address_count > MAX_ADDRESSES_PER_REQUEST && current.pending_revision.is_none() {
        return Err(
            "The wallet server must be updated to support batched address requests.".into(),
        );
    }
    if let Some(first) = first {
        // Timestamps can repeat within one second. The revision identifies the
        // pending snapshot, which determines which confirmed outputs are spendable.
        if first.indexed_height != current.indexed_height
            || first.indexed_hash != current.indexed_hash
            || first.pending_revision != current.pending_revision
        {
            return Err("Wallet data changed while loading. Please refresh.".into());
        }
    } else {
        *first = Some(current.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::api::test_support::{addresses, index, serve};

    fn response(request: &Value, activity: bool) -> Value {
        if activity {
            return json!({"used_addresses": request["addresses"], "index": index()});
        }
        let mut utxos = Vec::new();
        for address in request["addresses"].as_array().unwrap() {
            let number: u64 = address
                .as_str()
                .unwrap()
                .strip_prefix("address-")
                .unwrap()
                .parse()
                .unwrap();
            utxos.push(json!({
                "transaction_id": format!("{number:064x}"), "output_index": 0,
                "amount": 100_000_000, "script_pubkey": "5120", "block_height": 120,
                "confirmations": 1, "coinbase": false
            }));
        }
        json!({"utxos": utxos, "index": index()})
    }

    fn fetch(addresses: Vec<String>, url: &str, activity: bool) -> Result<usize, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            if activity {
                let used = fetch_used_addresses_from_server(addresses.clone(), url).await?;
                assert_eq!(used, unique_addresses(addresses));
                return Ok(used.len());
            }
            let utxos = fetch_utxos_from_server(addresses, url).await?;
            for utxo in &utxos {
                assert_eq!(utxo.amount, 100_000_000);
            }
            Ok(utxos.len())
        })
    }

    #[test]
    fn batches_addresses_at_the_server_limit_and_combines_every_result() {
        for activity in [true, false] {
            for count in [1_usize, 100, 101, 201] {
                let (url, server) = serve(count.div_ceil(100), move |_, request| {
                    (200, response(&request, activity))
                });
                assert_eq!(fetch(addresses(count), &url, activity).unwrap(), count);
                let requests = server.join().unwrap();
                let mut requested = Vec::new();
                for request in requests {
                    let batch = request["addresses"].as_array().unwrap();
                    assert!(batch.len() <= 100);
                    requested.extend(
                        batch
                            .iter()
                            .map(|address| address.as_str().unwrap().to_owned()),
                    );
                }
                assert_eq!(requested, addresses(count));
            }
        }
    }

    #[test]
    fn repeated_addresses_do_not_repeat_requests_or_inflate_balances() {
        for activity in [true, false] {
            let (url, server) = serve(1, move |_, request| (200, response(&request, activity)));
            let repeated = vec!["address-0".to_owned(); 101];
            assert_eq!(fetch(repeated, &url, activity).unwrap(), 1);
            assert_eq!(
                server.join().unwrap()[0]["addresses"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
    }

    #[test]
    fn rejects_partial_results_after_a_failed_or_changed_batch() {
        for activity in [true, false] {
            for change in ["error", "stale", "tip", "revision", "malformed"] {
                let (url, server) = serve(2, move |number, request| {
                    let mut body = response(&request, activity);
                    if number == 1 {
                        match change {
                            "error" => return (503, json!({"message": "Index unavailable"})),
                            "stale" => body["index"]["ready"] = json!(false),
                            "tip" => {
                                body["index"]["indexed_hash"] = json!("cd".repeat(32));
                                body["index"]["observed_node_hash"] = json!("cd".repeat(32));
                            }
                            "revision" => body["index"]["pending_revision"] = json!(8),
                            "malformed" => return (200, json!({})),
                            _ => unreachable!(),
                        }
                    }
                    (200, body)
                });
                assert!(fetch(addresses(101), &url, activity).is_err(), "{change}");
                server.join().unwrap();
            }
        }
    }

    #[test]
    fn old_servers_support_single_requests_but_cannot_safely_batch() {
        for activity in [true, false] {
            for count in [100, 101] {
                let (url, server) = serve(1, move |_, request| {
                    let mut body = response(&request, activity);
                    body["index"]
                        .as_object_mut()
                        .unwrap()
                        .remove("pending_revision");
                    (200, body)
                });
                assert_eq!(
                    fetch(addresses(count), &url, activity).is_ok(),
                    count == 100
                );
                server.join().unwrap();
            }
        }
    }

    fn output_page(start: usize, count: usize, cursor: Option<&str>) -> Value {
        let mut utxos = Vec::new();
        for number in start..start + count {
            utxos.push(json!({
                "transaction_id": format!("{number:064x}"), "output_index": 0,
                "amount": 100_000_000, "script_pubkey": "5120", "block_height": 120,
                "confirmations": 1, "coinbase": false
            }));
        }
        json!({"utxos": utxos, "next_cursor": cursor, "index": index()})
    }

    #[test]
    fn follows_all_output_pages_without_a_total_output_cap() {
        for count in [0_usize, 1_000, 2_000, 10_005] {
            let page_count = count.div_ceil(1_000).max(1);
            let (url, server) = serve(page_count, move |number, request| {
                assert_eq!(request["limit"], 1_000);
                assert_eq!(request["addresses"], json!(addresses(1)));
                if number == 0 {
                    assert!(request["cursor"].is_null());
                } else {
                    assert_eq!(request["cursor"], format!("page-{number}"));
                }
                let next = if number + 1 < page_count {
                    Some(format!("page-{}", number + 1))
                } else {
                    None
                };
                (
                    200,
                    output_page(
                        number * 1_000,
                        (count - number * 1_000).min(1_000),
                        next.as_deref(),
                    ),
                )
            });
            assert_eq!(fetch(addresses(1), &url, false).unwrap(), count);
            server.join().unwrap();
        }
    }

    #[test]
    fn finishes_each_address_batch_before_starting_the_next() {
        let (url, server) = serve(4, |number, request| {
            if number % 2 == 0 {
                assert!(request["cursor"].is_null());
                return (200, output_page(number, 1, Some("next-page")));
            }
            assert_eq!(request["cursor"], "next-page");
            (200, output_page(number, 1, None))
        });
        assert_eq!(fetch(addresses(101), &url, false).unwrap(), 4);
        let requests = server.join().unwrap();
        assert_eq!(requests[0]["addresses"], requests[1]["addresses"]);
        assert_eq!(requests[2]["addresses"], requests[3]["addresses"]);
        assert_eq!(requests[0]["addresses"].as_array().unwrap().len(), 100);
        assert_eq!(requests[2]["addresses"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn failed_or_invalid_pages_never_return_a_partial_balance() {
        for invalid in [
            "conflict",
            "error",
            "malformed",
            "stale",
            "revision",
            "tip",
            "duplicate",
            "order",
            "empty",
            "cursor-cycle",
            "cursor-length",
            "oversized",
        ] {
            let (url, server) = serve(2, move |number, _| {
                if number == 0 {
                    return (200, output_page(10, 1, Some("next-page")));
                }
                let mut body = output_page(11, 1, None);
                match invalid {
                    "conflict" => return (409, json!({"message": "Snapshot changed"})),
                    "error" => return (503, json!({"message": "Unavailable"})),
                    "malformed" => return (200, json!({})),
                    "stale" => body["index"]["ready"] = json!(false),
                    "revision" => body["index"]["pending_revision"] = json!(8),
                    "tip" => {
                        body["index"]["indexed_hash"] = json!("cd".repeat(32));
                        body["index"]["observed_node_hash"] = json!("cd".repeat(32));
                    }
                    "duplicate" => body = output_page(10, 1, None),
                    "order" => body = output_page(9, 1, None),
                    "empty" => body = output_page(0, 0, Some("page-3")),
                    "cursor-cycle" => body["next_cursor"] = json!("next-page"),
                    "cursor-length" => {
                        body["next_cursor"] = json!("a".repeat(MAX_UTXO_CURSOR_LENGTH + 1))
                    }
                    "oversized" => body = output_page(11, 1_001, None),
                    _ => unreachable!(),
                }
                (200, body)
            });
            assert!(fetch(addresses(1), &url, false).is_err(), "{invalid}");
            server.join().unwrap();
        }
    }

    #[test]
    fn empty_address_lists_need_no_server() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert!(fetch_used_addresses(Vec::new()).await.unwrap().is_empty());
            assert!(fetch_utxos(Vec::new()).await.unwrap().is_empty());
        });
    }
}
