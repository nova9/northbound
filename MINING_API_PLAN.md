# Mining API plan

Provide an optional solo-mining API for external miners. The miner builds the
coinbase transaction, chooses its reward address, and mines locally. No wallet
secrets are sent to the server. This is not a mining pool.

## Endpoints

| Endpoint | Purpose |
| --- | --- |
| `GET /v1/mining/template` | Return the node-generated block template. |
| `GET /v1/mining/block-hash?height=...` | Return the block hash at a validated height. |
| `POST /v1/mining/blocks` | Accept `block_hex` and submit the completed block to the node. |

Use `getblocktemplate` with fixed, supported rules and `submitblock` internally.
Keep general daemon RPC private. Preserve template fields required by miners.
The implemented request and response formats are documented in
[`server/MINING.md`](server/MINING.md). Review this contract with the miner author
before a public trial.

## Templates and long polling

- Maintain one shared template cache and one upstream watcher while mining is enabled.
- Fetch the first template normally. Allow clients to pass the last template
  identifier to wait for newer work, with a bounded timeout.
- Return immediately when newer work is already available. Otherwise, wait
  without blocking a thread. Clients keep mining while they wait.
- Notify all waiting clients when the shared template changes. Never open one
  daemon long-poll request per client.
- Refresh on a change to the node's best block hash, including reorganizations.
  Refresh periodically for mempool and timestamp changes using the node's template
  behavior. Do not rely on the index to detect a new mining tip.
- Coalesce concurrent refresh requests. Stop serving expired work if refresh fails;
  return a retryable error. Align watcher timeouts with the RPC client's timeout.
- Clients reconnect after responses or timeouts and back off after errors.

For the initial block-hash lookup, query the daemon. The index can supply these
hashes later if consistency with the template's chain is checked.

## Limits and results

- Disable mining endpoints by default; let each server operator opt in.
- Give mining separate request and concurrency limits from wallet traffic.
- Cap waiting clients, wait duration, and simultaneous block validations.
- Validate heights and block encoding. Set an explicit block submission size limit
  based on TrueNorth's block limits and hexadecimal encoding overhead.
- Let the node validate blocks. Preserve rejection and duplicate outcomes, and
  distinguish them from node failures without exposing internal details.
- Acceptance does not guarantee a permanent reward; chain status and coinbase
  maturity still apply.

## Implementation order

1. Define the API contract and fixed supported rules; review with the miner author
   before a public trial.
2. Add the node RPC calls and bounded HTTP endpoints.
3. Add the shared cache, watcher, and client long polling.
4. Test on isolated TrueNorth regtest following `TESTING.md`: mine and submit a
   real block, verify its reward, test invalid submissions, tip changes,
   reorganizations, timeouts, and multiple waiting clients.
5. Run formatting, compilation, and relevant tests. Measure server and node load
   before a small public trial.

No wallet UI changes are needed for this feature.
