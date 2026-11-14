# Indexed wallet API

Start the daemon, then run the server:

```sh
./scripts/regtest.sh start
cargo run -p server
curl -i http://127.0.0.1:3000/ready
```

The server starts one indexer worker and uses `.local/indexer/chain.sqlite` by
default. Set `TRUENORTH_INDEX_DATABASE` to choose another path; its parent is
created automatically. Stop a standalone indexer using that file before starting
the server. Existing version 1 indexes migrate automatically; hand-built tutorial
databases are deliberately not adopted.

`/ready` returns index status with HTTP 200 when fresh, or 503 during catch-up,
rollback, missing pending state, or stale observations. Wallet data requests also
return retryable JSON errors while unavailable. Startup rejects a wrong network,
wrong genesis, pruned daemon, or another writer. Permanent worker failure stops
the server. Initial node/cookie failures prevent startup.

| Endpoint | Source |
|---|---|
| `POST /v1/utxos` | Indexed confirmed outputs, excluding current pending spends |
| `POST /v1/addresses/activity` | Indexed confirmed and pending output history |
| `POST /v1/transactions` | Indexed confirmed and pending history, paginated |
| `POST /v1/transactions/status` | Index lookup, then daemon lookup if not yet observed |
| `GET /v1/chain/tip` | Daemon height, preserving the existing meaning |
| `GET /v1/fees/estimate` | Daemon fee estimate and policy floors |
| `POST /v1/transactions/validate` | Daemon mempool acceptance |
| `POST /v1/transactions/broadcast` | Daemon broadcast |

The first three requests take `addresses`. History additionally accepts optional
`limit` (1–200) and `cursor`, and returns `next_cursor`. HTTP 409 means discard
partial pages and restart. All three responses include `index` metadata with
indexed/observed tips, node contact, pending observation, readiness, and the
30-second freshness limit. An indexed snapshot can become outdated after it is
observed; broadcasting does not imply the next index snapshot already contains it.

Errors use `{"message":"..."}`. Invalid addresses/request limits return 400,
unavailable index data returns 503, and node failures return 502. Internal database
details stay in server logs. Read-only SQLite requests run on blocking threads,
with at most 16 concurrent queries and a 3-second SQLite busy timeout.

Other settings remain `TRUENORTH_NETWORK` (`regtest` or `main`),
`TRUENORTH_RPC_URL`, `TRUENORTH_RPC_COOKIE_PATH`, and `TRUENORTH_SERVER_BIND`.
`TRUENORTH_INDEX_POLL_SECONDS` defaults to 2 and accepts 1–10 in server mode.

Container deployments should mount persistent writable storage and set
`TRUENORTH_INDEX_DATABASE` there. The image defaults to
`/home/northbound/indexer/chain.sqlite`. Use one server replica per database on a
local filesystem; the writer lock does not provide a distributed deployment mode.
The Kubernetes example mounts a separate index PVC and probes `/ready`; its 5 GiB
claim is an example size, not a capacity guarantee. The NixOS service uses an
owned `/var/lib/northbound` state directory. Deployment image/source pins remain
managed independently and must be updated when deploying the new implementation.

Run the complete HTTP integration test with an isolated node and database:

```sh
./scripts/test-truenorth-integration.sh server
```

See the [indexer documentation](../indexer/README.md) for storage, query semantics,
recovery, operating limits, and known limitations.
