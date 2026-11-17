# Northbound 0.2.0-alpha.3

Changes since `v0.2.0-alpha.2`.

This release improves transaction broadcast errors, confirmation lookups, and
server network discovery. It also fixes a node shutdown error that could stop
the API server.

## API changes

### Actionable broadcast responses

`POST /v1/transactions/broadcast` keeps its existing request and success formats.

- Transaction refusals return HTTP 400 with the node's rejection reason.
- Missing/spent inputs and applicable mempool conflicts return HTTP 409 with the
  node's rejection reason.
- Repeated broadcasts of accepted transactions return HTTP 200 with their exact
  transaction ID, while pending or confirmed. Confirmed transactions whose
  outputs are all spent are recognized through the server index.
- Unreachable nodes, malformed responses, and unrelated RPC errors remain HTTP
  502 with a generic message. An unavailable index during the spent-output
  repeat check returns HTTP 503.

Error bodies remain `{"message":"..."}`. After a broadcast error, check the
transaction status before treating the payment as failed or sending it again.

### Confirmation height in transaction status

`POST /v1/transactions/status` adds nullable `block_height` alongside `known`:

| State | Response |
| --- | --- |
| Unknown | `{"known":false,"block_height":null}` |
| Pending | `{"known":true,"block_height":null}` |
| Confirmed | `{"known":true,"block_height":120}` |

The lookup works independently of history pagination and unspent change outputs.
It checks the server index first, then falls back to the daemon. A failed lookup
returns an HTTP error rather than claiming the transaction is unknown. The daemon
fallback needs a synced `txindex` for historical lookups; locally indexed
transactions do not require it. An unknown result alone does not establish that
an earlier broadcast failed.

### Network identity in server info

`GET /info` now returns the server's configured network alongside its version:

```json
{"version":"0.2.0-alpha.3","network":"main"}
```

`network` is `main` or `regtest`, checked against the daemon at startup. This
endpoint remains available without a live node or ready index. The chain-tip
response remains height-only: `{"height":120}`.

Clients with strict response schemas must allow the added `network` and
`block_height` fields. Clients supporting older servers should tolerate their
absence. See [the API documentation](server/README.md) for the full contract.

## Wallet behavior

- Broadcast rejection reasons remain visible to the user.
- The wallet retains status checks after broadcast errors: a known transaction
  is treated as sent; unknown or failed checks leave the outcome uncertain.
- Transactions are not automatically retried. Older status responses without
  `block_height` remain readable.

## Server and development changes

- Preserve structured RPC errors, including method, numeric code, and message,
  even on unsuccessful HTTP responses. Only broadcast rejection reasons are
  exposed publicly.
- Treat non-JSON HTTP failures as transport errors so the indexer uses its
  existing retry path during node shutdown or overload. Malformed successful
  responses remain invalid-data errors.
- Verify that mining rewards retain the existing history `coinbase` flag after
  their outputs are spent.
- Extend isolated regtest coverage for repeated broadcasts, conflicting spends,
  confirmation heights, and payments without change. The server integration
  command automatically tests with daemon `txindex` both disabled and enabled.
