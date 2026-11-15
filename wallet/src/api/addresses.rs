use std::collections::HashSet;

use shared::{
    AddressActivityRequest, AddressActivityResponse, IndexStatus, UtxoRequest, UtxoResponse,
    WalletUtxo,
};

use super::{endpoint, http_client, require_current_index, require_success};

const MAX_ADDRESSES_PER_REQUEST: usize = 100;

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
        let request = UtxoRequest {
            addresses: batch.to_vec(),
        };
        let response = client
            .post(url)
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("could not contact wallet server: {error}"))?;
        let response = require_success(response, "Could not load spendable outputs").await?;
        let response = response.json::<UtxoResponse>().await.map_err(|error| {
            format!("wallet server returned invalid spendable outputs: {error}")
        })?;
        require_matching_snapshot(&mut snapshot, &response.index, addresses.len())?;
        for utxo in response.utxos {
            let outpoint = (utxo.transaction_id.clone(), utxo.output_index);
            if !seen_outpoints.insert(outpoint) {
                return Err("The wallet server returned a duplicate spendable output.".into());
            }
            utxos.push(utxo);
        }
    }
    Ok(utxos)
}

fn unique_addresses(addresses: Vec<String>) -> Vec<String> {
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::*;

    fn index() -> Value {
        json!({
            "ready": true, "indexed_height": 120, "indexed_hash": "ab".repeat(32),
            "observed_node_height": 120, "observed_node_hash": "ab".repeat(32),
            "last_node_contact": 1000, "pending_available": true,
            "pending_observed_at": 1000, "pending_revision": 7, "max_age_seconds": 30
        })
    }

    fn addresses(count: usize) -> Vec<String> {
        (0..count)
            .map(|number| format!("address-{number}"))
            .collect()
    }

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

    fn serve(
        requests: usize,
        reply: impl Fn(usize, Value) -> (u16, Value) + Send + 'static,
    ) -> (String, thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let thread = thread::spawn(move || {
            let mut received = Vec::new();
            for number in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let body_start = loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break position + 4;
                    }
                };
                let headers = String::from_utf8(bytes[..body_start].to_vec()).unwrap();
                let content_length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            return Some(value.trim().parse().unwrap());
                        }
                        None
                    })
                    .unwrap();
                while bytes.len() < body_start + content_length {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let request: Value = serde_json::from_slice(&bytes[body_start..]).unwrap();
                received.push(request.clone());
                let (status, body) = reply(number, request);
                let body = body.to_string();
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            received
        });
        (url, thread)
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
