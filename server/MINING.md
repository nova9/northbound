# Solo-mining API

Mining routes are disabled by default. Enable them explicitly:

```sh
TRUENORTH_MINING_ENABLED=true cargo run -p server
```

`TRUENORTH_MINING_REQUESTS_PER_SECOND` sets a separate mining request budget
(default 100). Wallet requests keep their existing budget. Both still share the
same process and daemon; these limits do not guarantee resource isolation.

## Get a template

```http
GET /v1/mining/template
```

Returns the complete `getblocktemplate` result with `rules: ["segwit"]`, including
Core's `longpollid`. Required consensus rules and all transaction data are
preserved. The miner must understand the returned rules, construct its own
coinbase and commitments, and select the RandomX seed using TrueNorth's rules.

To wait for newer work, pass the previous identifier:

```http
GET /v1/mining/template?longpollid=<previous-longpollid>
```

A different identifier returns immediately. Otherwise, the request waits up to
25 seconds, then returns the latest fresh template even if unchanged. Clients
should allow more than 25 seconds for their HTTP timeout, reconnect after each
response, and back off on failures. Unknown query parameters are rejected.

One upstream watcher polls the daemon every second and serializes a shared
response once per refresh. Tip changes, including reorganizations, reach clients
on the next successful poll; this is not an immediate tip notification. Core
controls mempool-driven template rebuilding and refreshes timestamps. Timestamp
changes alone do not finish waiting requests with the same identifier.

Upstream refreshes time out after 3 seconds. Failed refreshes clear the cache;
cached work also expires after 5 seconds without a successful refresh. Missing
or expired work returns 503. There are at most 256 concurrent template requests.
One failed refresh may therefore interrupt waiting clients; they should retry
with backoff.

## Look up a seed block hash

```http
GET /v1/mining/block-hash?height=10000
```

```json
{"height":10000,"hash":"<64 hexadecimal characters>"}
```

Queries the node's active chain. Heights must be integers between 0 and
2,147,483,647; unavailable heights return 400. At most eight lookups run at once.
Templates and lookups are separate snapshots: a reorganization between calls can
change the chain. Miners should obtain fresh work after a tip change.

## Submit a completed block

```http
POST /v1/mining/blocks
Content-Type: application/json

{"block_hex":"<entire serialized block in hexadecimal>"}
```

The server checks size and encoding, then calls `submitblock`. It never changes
the reward address or block contents. One submission is allowed at a time, with
the slot acquired before buffering the body. Block hex is capped at 8,000,000
characters (Core's 4,000,000-byte serialized block bound). The JSON body limit
allows another 1,024 bytes for field names and whitespace.
The complete submission request has a 30-second timeout, including upload. A
timeout does not prove that the node rejected the block; check its status before
retrying, since a retry may report a duplicate.

HTTP 200 means the node returned a block-validation outcome:

```json
{"status":"accepted","result":null}
{"status":"duplicate","result":"duplicate"}
{"status":"rejected","result":"bad-cb-amount"}
{"status":"inconclusive","result":"inconclusive"}
```

The original result string is preserved. `duplicate-invalid` is rejected;
`duplicate-inconclusive` is inconclusive. Acceptance does not guarantee active-chain
inclusion or a permanent reward. Coinbase maturity still applies.

Malformed input returns 400 (missing or unexpected JSON fields may return 422),
oversized bodies 413, rate exhaustion 429 with `Retry-After: 1`, unavailable
capacity/work 503, and upstream failures 502. Errors use `{"message":"..."}`.
General daemon RPC and credentials remain private.

## Verification

```sh
cargo test -p server --lib
cargo test -p truenorth-rpc --lib
./scripts/test-truenorth-integration.sh server
```

The isolated regtest scenario covers opt-in startup, template and hash lookup,
real RandomX block submission and reward output, duplicate handling, long-poll
updates, reorganizations, and node failure. Unit tests cover timeout, cache
expiration, fan-out, malformed inputs, limits, and node outcome translation.

Review this contract with the miner author and measure daemon/server load before
a public trial. No production load test is part of the integration suite.
