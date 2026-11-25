use serde_json::{Value, json};
use shared::MAX_UTXO_CURSOR_LENGTH;

use super::{ServerClient, unique_addresses};
use crate::server::test_support::{addresses, index, serve};

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
    let client = ServerClient::new(url).unwrap();
    runtime.block_on(async {
        if activity {
            let used = client.fetch_used_addresses(addresses.clone()).await?;
            assert_eq!(used, unique_addresses(addresses));
            return Ok(used.len());
        }
        let utxos = client.fetch_utxos(addresses).await?;
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
    let client = ServerClient::new("http://127.0.0.1:1").unwrap();
    runtime.block_on(async {
        assert!(
            client
                .fetch_used_addresses(Vec::new())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(client.fetch_utxos(Vec::new()).await.unwrap().is_empty());
    });
}
