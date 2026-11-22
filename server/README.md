# Indexed wallet API

An optional, separately limited [solo-mining API](MINING.md) provides templates,
long polling, block-hash lookup, and completed block submission. It is disabled by default.

Start the daemon, then run the server:

```sh
./scripts/regtest.sh start
cargo run -p server
curl -i http://127.0.0.1:3000/ready
```

The server starts one indexer worker and uses `.local/indexer/chain.sqlite` by
default. Set `TRUENORTH_INDEX_DATABASE` to choose another path; its parent is
created automatically. Existing version 1 indexes migrate automatically; hand-built
tutorial databases are deliberately not adopted.

`/ready` returns index status with HTTP 200 when fresh, or 503 during catch-up,
rollback, missing pending state, or stale observations. Wallet data requests also
return retryable JSON errors while unavailable. Startup rejects a wrong network,
wrong genesis, pruned daemon, or another writer. Permanent worker failure stops
the server. Initial node/cookie failures prevent startup.

| Endpoint | Source |
|---|---|
| `GET /info` | Server release version and configured network |
| `POST /v1/utxos` | Paginated confirmed outputs, excluding current pending spends |
| `POST /v1/addresses/activity` | Indexed confirmed and pending output history |
| `POST /v1/transactions` | Indexed confirmed and pending history, paginated |
| `POST /v1/transactions/status` | Index lookup, then daemon lookup if not yet observed |
| `GET /v1/chain/tip` | Validated daemon height |
| `GET /v1/fees/estimate` | Daemon fee estimate and policy floors |
| `GET /v2/fees/estimate` | Precise daemon fee estimate and policy floors in atomic units/kvB |
| `POST /v1/transactions/validate` | Daemon mempool acceptance |
| `POST /v1/transactions/broadcast` | Daemon broadcast |

`GET /info` returns `{"version":"0.1.0","network":"main"}`. `version` uses the
running server's Cargo package version; `network` is the configured `main` or
`regtest` network, checked against the daemon at startup. Clients can check this
before sending addresses or preparing a payment. It requires no node or index
query and uses the normal API rate limit. A 404 from an older server means version
discovery is unavailable; older responses may omit `network`.

`GET /v1/chain/tip` returns `{"height":120}`. The daemon's network is still checked
against server configuration on this request; a mismatch returns 502.

`POST /v1/transactions/status` takes `{"transaction_id":"..."}` and returns:

| State | `known` | `block_height` |
|---|---|---|
| Unknown | `false` | `null` |
| Pending | `true` | `null` |
| Confirmed | `true` | Height of the containing block |

The lookup uses a fresh index snapshot, independent of history pagination and
whether any transaction outputs remain unspent. If absent from the snapshot, it
asks the daemon for the transaction and, when confirmed, its active block header.
A missing or synchronizing index returns 503. A failed daemon lookup returns 502;
without a synced daemon `txindex`, an absent mempool transaction cannot establish
absence from the blockchain and also returns 502. This does not affect confirmed
transactions already in the server index. Older servers omit `block_height`, so
clients should tolerate its absence. Results describe the observed chain and may
change after a reorganization.

`POST /v1/transactions/broadcast` takes `{"transaction_hex":"..."}` and returns
`{"transaction_id":"..."}` with HTTP 200 for accepted transactions, including
repeated submissions while pending or confirmed. TrueNorth's already-confirmed
code (-27) is success; the daemon decodes the submitted transaction to recover its
exact ID. If all its outputs have been spent and the daemon instead reports
missing inputs, the server checks that exact ID in the fresh confirmed index
before reporting success. A conflicting transaction is never success.

Broadcast error mapping (the JSON body remains `{"message":"..."}`):

| Condition | HTTP status | Message |
|---|---|---|
| Invalid hex or malformed transaction (-22) | 400 | Input validation or node rejection reason |
| Transaction refusal (-25, -26) | 400 | Node rejection reason |
| Missing/spent inputs or mempool conflict | 409 | Node rejection reason |
| Node unreachable, malformed response, unrelated RPC error | 502 | Generic upstream error |
| Index unavailable during the spent-output repeat check | 503 | Index readiness error |

Only `sendrawtransaction` rejection reasons are exposed; credentials and unrelated
node infrastructure errors stay private. After any broadcast error, clients should
check transaction status: `known: true` means it was sent. An unknown result or a
failed status check leaves an ambiguous send unresolved. The desktop wallet keeps
the rejection reason visible and never automatically retries the transaction.

The first three requests take `addresses` (at most 100 per request). History also
accepts optional `limit` (1–200) and `cursor`, and returns `next_cursor`.
All three responses include `index` metadata with
indexed/observed tips, node contact, pending observation, readiness, and the
30-second freshness limit. An indexed snapshot can become outdated after it is
observed; broadcasting does not imply the next index snapshot already contains it.

The desktop wallet batches UTXO, address activity, and history requests into groups of 100
addresses. It accepts combined results only when every batch has the same indexed
tip and `pending_revision`. This revision changes whenever the indexer publishes
a pending snapshot; observation timestamps alone cannot identify a snapshot.
If the snapshot changes, the wallet discards the results and asks the user to
refresh. History restarts the entire load once before asking the user to refresh.

`POST /v1/transactions` returns `transactions`, `next_cursor`, and `index`.
Each transaction has this structure:

```json
{
  "transaction_id": "...",
  "net_amount": -310000000,
  "network_fee": 10000000,
  "coinbase": false,
  "block_height": 120,
  "confirmations": 1,
  "timestamp": 1794700000
}
```

Amounts are integer atomic units (100,000,000 per NORTH). `net_amount` is outputs
to requested addresses minus inputs from them; positive values increase wallet
holdings and negative values decrease them. Fee effects are already included.
`network_fee` is the entire transaction's fee, independent of address ownership,
and is null for coinbase transactions. It does not claim this wallet paid all of
the fee. `block_height` is null and confirmations are zero while pending;
`timestamp` is the block time when confirmed or first-seen time while pending.

`coinbase` identifies mining rewards explicitly. The wallet labels these as
"Mining reward" and shows "Immature" below 100 confirmations. Older servers omit
this field, so update the server as well as the wallet to enable these labels.
The flag remains on history entries after every reward output has been spent.

Fetch every page of each disjoint address batch, group entries by transaction ID,
and sum only `net_amount`. Keep matching transaction metadata and the fee once;
never sum or subtract the fee again. The wallet sorts the combined history with
pending entries first, then descending block height and ascending transaction ID.
It has no total history cap across pages. This replaces the old amount/address
fields; deploy the updated wallet and server together.

`POST /v1/utxos` always returns a page. It accepts optional `limit` (1–10,000,
default 1,000) and `cursor` fields alongside `addresses`:

```json
{"addresses":["rnorth..."],"limit":1000,"cursor":null}
```

The response contains `utxos`, `next_cursor`, and `index`. Repeat the same
addresses with the returned cursor until `next_cursor` is null; only then is the
result complete. Outputs are ordered by block height, transaction ID, and output
index. There is no total output cap across pages. A cursor is tied to the chain
tip and pending snapshot revision. HTTP 409 means discard every page and address
batch and restart; never use a partial result as a balance. The desktop wallet
loads all pages in each address batch and rejects changes between responses.
Deploy the updated wallet and server together; callers must follow pagination.

`GET /v2/fees/estimate` returns `{"atomic_units_per_kvbyte":1001,"target_blocks":6}`,
for example. One kvB is 1,000 virtual bytes. The rate is the highest of Core's
six-block estimate, mempool minimum, and relay minimum, converted exactly from
NORTH/kvB to integer atomic units/kvB. If the estimate is unavailable, the policy
minimums are used. Calculate the fee as `ceil(vbytes * atomic_units_per_kvbyte / 1000)`
using checked or widened integer arithmetic. The wallet uses this endpoint and
rounds only the final fee; any dust change is still added to the fee.
`/v1/fees/estimate` retains its existing `atomic_units_per_vbyte` field and rounds
the rate up to whole atomic units/vB. Other endpoints remain on `/v1`.

Errors use `{"message":"..."}`. Invalid addresses/request limits return 400,
unavailable index data returns 503, and node failures return 502. Internal database
details stay in server logs. Read-only SQLite requests run on blocking threads,
with at most 16 concurrent queries and a 3-second SQLite busy timeout.

The API accepts 100 requests per one-second window by default, shared across all
clients and endpoints in one server process. Set
`TRUENORTH_SERVER_REQUESTS_PER_SECOND` to a positive integer to change this limit.
Excess requests return HTTP 429 with a JSON error and `Retry-After: 1`; clients
should wait at least one second before retrying. Rejected requests are not queued
or sent to the database or daemon. Invalid requests also consume the budget.
`/ready` is exempt so health probes continue to work during heavy traffic.
This is a fixed-window limit: bursts can occur on either side of a window
boundary. It is independent of the 16 concurrent index query limit, and does not
provide per-client fairness or a shared budget across server processes.

Other settings remain `TRUENORTH_NETWORK` (`regtest` or `main`),
`TRUENORTH_RPC_URL`, `TRUENORTH_RPC_COOKIE_PATH`, and `TRUENORTH_SERVER_BIND`.
`TRUENORTH_INDEX_POLL_SECONDS` defaults to 2 and accepts 1–10 in server mode.

The [NixOS configuration](devops/app.nix) runs the server and daemon as systemd
services and configures Caddy for HTTPS. The server stores its index in an owned
`/var/lib/northbound` state directory; the daemon stores its chain in
`/var/lib/truenorth`. Use one server process per database on a local filesystem;
the writer lock does not provide a distributed deployment mode. Use `/ready`
to check index readiness.

Follow the [server deployment instructions](../README.md#server-deployment)
to pin the GitHub release tag and checksum. Nix downloads and verifies the binary.
Daemon RPC stays on loopback, with cookie access granted to the server's user.

Run the complete HTTP integration test with an isolated node and database:

```sh
./scripts/test-truenorth-integration.sh server
```

See the [indexer documentation](../indexer/README.md) for storage, query semantics,
recovery, operating limits, and known limitations.
