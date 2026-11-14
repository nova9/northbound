# SQLite indexer and queries

This crate indexes confirmed TrueNorth blocks and coherent mempool snapshots.
It provides read-only UTXO, script activity, transaction history, and transaction
lookup functions. The API server runs its worker on a dedicated thread and uses
these queries instead of request-time blockchain scans. RPC transport and exact
block/transaction decoding live in the separate `truenorth-rpc` crate.

## Run

Normally, run `cargo run -p server`; it starts the indexer automatically with
`.local/indexer/chain.sqlite` as its default database. Stop any standalone writer
using that database first. For a standalone catch-up, start an unpruned regtest
daemon with `./scripts/regtest.sh start`, then:

```sh
mkdir -p .local/indexer
TRUENORTH_INDEX_DATABASE="$PWD/.local/indexer/chain.sqlite" \
  cargo run -p indexer -- --once
```

`--once` reconciles the saved index, publishes a mempool snapshot, and exits. Omit
it to poll continuously; Ctrl-C stops the process. Reusing the database resumes
from its highest committed block rather than rescanning genesis. Open the file
in TablePlus to inspect the records; avoid editing it while the indexer runs.

| Environment variable | Default |
|---|---|
| `TRUENORTH_INDEX_DATABASE` | Required path; parent directory must exist |
| `TRUENORTH_NETWORK` | `regtest`; also accepts `main` |
| `TRUENORTH_RPC_URL` | `http://127.0.0.1:18443`; main uses port `9554` |
| `TRUENORTH_RPC_COOKIE_PATH` | `.local/truenorth/regtest/.cookie`; main uses `.local/truenorth/.cookie` |
| `TRUENORTH_INDEX_POLL_SECONDS` | `2`; accepts 1–3600 |

Use a new database file or an existing index created by this crate. Version 1
indexes migrate to version 2 without rescanning confirmed blocks. Manually
populated databases are rejected without modifying their records. The indexer uses a schema version
and application ID, verifies the network and genesis identity, enables foreign
keys on its connection, and uses WAL with full synchronization.

## Storage and reorganizations

`migrations/001_index.sql` defines blocks, transactions, outputs, spends, and a
singleton chain identity. Hashes use RPC display byte order; scripts preserve
all bytes. Monetary values are signed 64-bit integer atomic units, decoded from
JSON with decimal precision preserved. Transaction IDs must be unique on the
active chain; an unexpected duplicate fails the block instead of overwriting it.

Each block is fetched and decoded before its SQLite write transaction begins.
The write rechecks the indexed predecessor, resolves inputs in transaction
order, validates eligibility and coinbase maturity, checks monetary totals,
and commits all records together. Missing inputs, double spends, invalid
amounts, and SQL errors leave the previous tip intact. The indexed tip is derived
from the highest block row, so progress cannot commit separately from its data.

Each synchronization step compares our tip with the node's active hash at the
same height. Mismatches remove one tip block at a time until a common ancestor
is reached. Block deletion cascades through owned transactions, outputs, and
spends. Previous-outpoint foreign keys use `NO ACTION`, preventing a surviving
spend from losing its referenced output. Replacement blocks are inserted in
order; their parent and active-chain hash are checked again before writing.

One OS file lock is held on `<database>.writer.lock` for the writer's lifetime.
A second indexer using that database path is refused. The OS releases the lock
on process exit, including crashes; the lock file itself can remain. Keep the
database on a local filesystem, use one canonical file without hard-link aliases,
and do not remove its lock file while a writer is running. SQLite still enforces
its own write transactions, but the sidecar lock coordinates indexer processes,
not arbitrary SQL clients.

RPC calls time out after 30 seconds and responses are bounded to 64 MiB. During
synchronization, RPC errors trigger five retries with exponential delays capped
at 32 seconds, then stop the command. Startup errors stop immediately. Neither
path fabricates empty data or advances progress after a failed RPC. Decode,
identity, and database errors stop immediately. A missing historical block
must be made available before retrying; pruned nodes are rejected.

An indexed tip is an observation, not a promise that the node cannot reorganize
immediately afterward. Manual edits are unsupported: tip comparison detects
branch changes, not arbitrary missing or modified records.

## Pending transactions and readiness

Migration `002_pending.sql` adds pending transactions, outputs, spends, and
snapshot status. The worker captures the node tip and mempool sequence, fetches
decoded transactions (reusing unchanged cached transactions), orders parents
before children, and rechecks the tip and sequence before publishing atomically.
A snapshot is never mixed with confirmed data from a different base block.
Block application and rollback invalidate readiness until pending data is rebuilt.
Eviction and replacement remove the previous pending effects.

Readiness requires equal indexed/observed tips, a matching pending base, and
node contact and pending observation no older than **30 seconds**. RPC failure
marks the index unavailable immediately. Data queries reject unavailable state
instead of returning an empty wallet. A standalone `--once` snapshot also expires
after 30 seconds without further observations. The server limits its polling
interval to 1–10 seconds and stops if the worker fails permanently.

## Query functions

```rust,ignore
let scripts = indexer::decode_addresses(&addresses, "regtest")?;
let mut reader = indexer::IndexReader::open(&database_path, "regtest")?;
let snapshot = reader.read_snapshot(&scripts)?;
let utxos = snapshot.query_utxos()?;
let activity = snapshot.query_script_activity()?;
let history = snapshot.query_transaction_history(None, 100)?;
```

Address decoding supports TrueNorth witness programs, including P2QRH, and its
mainnet/regtest Base58 prefixes. Invalid checksums and wrong-network addresses
are rejected locally. Scripts are deduplicated, with at most 100 per request.

One read transaction supplies results and indexed/observed metadata. UTXOs are
confirmed, eligible outputs with neither confirmed nor pending spends. Pending
outputs are history/activity only. Coinbase flags and confirmations are returned
without hiding immature outputs; the caller must check maturity before signing.
UTXO requests are bounded to 10,000 outputs and fail rather than silently truncate.

Activity includes spent outputs. History deduplicates transactions across scripts
and reports change as received value while outgoing amounts exclude owned outputs.
Fees use all inputs minus all outputs and are attributed only when all inputs
belong to the requested wallet scripts. With mixed ownership, fee and recipient
are unknown and `sent_amount` is the wallet's net debit, including any unknown fee.

History uses keyset pagination (1–200 transactions, default 100) with a cursor
bound to the indexed tip hash and pending snapshot revision. Any tip change,
including normal growth, or pending snapshot replacement rejects old cursors;
restart from page one. There are no OFFSET queries. The wallet follows pages,
restarts once on a changed snapshot, and bounds its display to 10,000 transactions.

Query-plan inspection confirms script-index seeks and indexed outpoint/transaction
lookups, starting from the requested scripts. History still processes matching
wallet activity to order each page. Large histories and mempools need workload
measurements; there is no capacity or query-latency guarantee. Pending snapshots
currently accept at most 100,000 transactions.

## Verification

```sh
cargo fmt -p indexer -- --check
cargo check -p indexer
cargo test -p indexer
cargo test -p indexer --lib
./scripts/test-truenorth-integration.sh indexer
./scripts/test-truenorth-integration.sh server
```

The integration harness creates an isolated daemon and removes its temporary
chain afterward. Invalidation and reconsideration only target that test node.
The default harness invocation still runs wallet tests.

SQLite-only checks live in inline `#[cfg(test)] mod tests` blocks at the end of
their implementation files, following the [workspace convention](../TESTING.md).
They run with `cargo test -p indexer --lib` without a daemon. The daemon integration test remains
in `tests/truenorth_regtest.rs` and runs through the isolated shell harness above.

The regtest test verifies P2QRH scripts and amounts against `validateaddress`
and `gettxout`, restart, same-height branch replacement, removal and subsequent
reconfirmation of a payment, rollback of more than fifty blocks, node failure,
wrong-network rejection, executable catch-up, and equality with a clean rebuild.
It also compares the full indexed UTXO result with `scantxoutset` after mining,
payment/change, restart, and reorganizations, including a deep rollback. The
comparison covers outpoints, exact atomic amounts, scripts, block heights,
confirmations, and coinbase flags across multiple scripts, a duplicate address,
and an unused address. Scan amounts are parsed independently of the production
decoder. When pending spends exist, their outpoints are obtained from the daemon
and excluded from the scan to match the indexer's deliberate pending-spend filter.
SQLite tests cover same-block parent/child spends, all-or-nothing failed writes,
abrupt exits before and after commit, exclusive writer ownership, reader snapshot
consistency, malformed data, duplicate IDs, and invalid spends.

The server regtest test runs with `blockfilterindex=0` and no `txindex`, exercising
pending receives, parent/child spends, RBF eviction, confirmation, reorganization,
pagination invalidation, restart, and node failure through HTTP endpoints.

Consensus-specific constants were checked against TrueNorth **v30.5.4** source:

- [`ConnectBlock`](https://github.com/truenorth-project/truenorth/blob/v30.5.4/src/validation.cpp)
  skips connecting genesis transactions; genesis outputs remain history only.
- [`CScript::IsUnspendable`](https://github.com/truenorth-project/truenorth/blob/v30.5.4/src/script/script.h)
  excludes scripts beginning with `OP_RETURN` or exceeding 10,000 bytes.
- [`COINBASE_MATURITY`](https://github.com/truenorth-project/truenorth/blob/v30.5.4/src/consensus/consensus.h)
  is 100; maturity is measured at the spending block height.
- [`MAX_MONEY`](https://github.com/truenorth-project/truenorth/blob/v30.5.4/src/consensus/amount.h)
  is 10 billion NORTH, or 10^18 atomic units, which fits signed 64-bit storage.

The client is pinned to [rusqlite 0.42.0](https://docs.rs/rusqlite/0.42.0/rusqlite/)
with bundled SQLite. The daemon remains responsible for full consensus validation;
this indexer does not independently verify signatures or proof of work.
