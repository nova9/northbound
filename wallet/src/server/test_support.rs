use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

pub fn index() -> Value {
    json!({
        "ready": true, "indexed_height": 120, "indexed_hash": "ab".repeat(32),
        "observed_node_height": 120, "observed_node_hash": "ab".repeat(32),
        "last_node_contact": 1000, "pending_available": true,
        "pending_observed_at": 1000, "pending_revision": 7, "max_age_seconds": 30
    })
}

pub fn addresses(count: usize) -> Vec<String> {
    (0..count)
        .map(|number| format!("address-{number}"))
        .collect()
}

pub fn serve(
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
                .unwrap_or(0);
            while bytes.len() < body_start + content_length {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            let request: Value = if content_length == 0 {
                json!(headers.lines().next().unwrap())
            } else {
                serde_json::from_slice(&bytes[body_start..]).unwrap()
            };
            received.push(request.clone());
            let (status, body) = reply(number, request);
            let body = body.to_string();
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        received
    });
    (url, thread)
}
