# Northbound 0.2.0-alpha.2

Changes since `v0.2.0-alpha.1`.

Mining rewards now have their own history label and an immature status. Server
operators can optionally enable a solo-mining API for external miners.

## API changes

### Transaction history: new `coinbase` field

`POST /v1/transactions` adds a boolean `coinbase` field to each transaction:

```json
{
  "transaction_id": "<transaction ID>",
  "net_amount": 51200000000,
  "network_fee": null,
  "coinbase": true,
  "block_height": 12345,
  "confirmations": 68,
  "timestamp": 1794787200
}
```

- `true` identifies a mining reward; `false` identifies a normal transaction.
- Amounts remain integer atomic units. The example represents 512 NORTH.
- `network_fee` remains null for mining rewards.
- Existing request formats, pagination, and amount/fee meanings are unchanged.
- API clients with strict response schemas must allow the new field.

**Upgrade the server as well as the wallet to see mining labels.** An older
server omits `coinbase`; the new wallet still loads its history but displays
those rewards using the ordinary transaction labels. It does not infer mining
rewards from the amount or a null fee.

### New optional solo-mining endpoints

| Method and path | Behavior |
| --- | --- |
| `GET /v1/mining/template` | Returns the complete node-generated block template. |
| `GET /v1/mining/template?longpollid=...` | Waits for a changed template identifier, for at most 25 seconds. |
| `GET /v1/mining/block-hash?height=...` | Returns `{"height":...,"hash":"..."}` from the node's active chain. |
| `POST /v1/mining/blocks` | Accepts `{"block_hex":"..."}` and submits the completed block to the node. |

These routes are **disabled by default** and return 404 until enabled:

```sh
TRUENORTH_MINING_ENABLED=true cargo run -p server --release
```

The example enables mining; retain the network, RPC, and database environment
settings for your deployment. `TRUENORTH_MINING_REQUESTS_PER_SECOND` controls a
separate mining request budget, defaulting to 100 requests/second.

Templates share one upstream watcher and cache. Long-poll clients should use an
HTTP timeout longer than 25 seconds and reconnect after every response. A timeout
can return the same template identifier. Missing or stale work returns 503.

Block submission returns `status` and the original node `result`:

```json
{"status":"accepted","result":null}
```

**HTTP 200 does not always mean the block was accepted.** Inspect `status`, which
can also be `duplicate`, `rejected`, or `inconclusive`. Invalid requests, rate
exhaustion, capacity limits, and upstream failures use HTTP error responses.

This is solo mining: the miner constructs its reward transaction and mines
locally. The server does not distribute pool payouts or receive wallet secrets.
See [the mining API contract](server/MINING.md) for limits, errors, and examples.

## Wallet changes

- Mining rewards appear as **Mining reward** in history.
- Rewards below 100 confirmations show **Immature**, with an explanation that
  they are excluded from available balance until maturity.
- Mature rewards show **Confirmed**. The existing spendability rule is unchanged.
- Improve Linux window/menu behavior and overview balance error display.

## Server and development changes

- Consolidate indexer retry handling.
- Add `scripts/mainnet.sh` with `start`, `stop`, `tip`, and `peers` commands.
  Mainnet and regtest daemon chain data use separate network directories;
  configure a separate mainnet API index database explicitly.
- Add k6 API load-testing tooling and isolated mining API regtest coverage.
- Remove the repository's Docker/Kubernetes deployment examples. NixOS deployment
  remains documented in [the server README](server/README.md).
