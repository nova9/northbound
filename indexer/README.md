# Standalone SQLite indexer

This crate indexes confirmed TrueNorth blocks independently of the API server.
It does not change server routes or wallet behavior. Mempool snapshots, wallet
queries, API freshness responses, and pagination are not implemented here yet.

## Run

Start a local unpruned regtest daemon with `./scripts/regtest-start.sh`, then:

```sh
mkdir -p .local/indexer
TRUENORTH_INDEX_DATABASE="$PWD/.local/indexer/chain.sqlite" \
  cargo run -p indexer -- --once
```

`--once` reconciles the saved index with the node, catches up, and exits. Omit
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

Use a new database file. Existing manually populated databases are rejected
without migrating or deleting their records. The indexer uses a schema version
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
immediately afterward. There is no API readiness or pending-state claim yet.

## Verification

```sh
cargo fmt -p indexer -- --check
cargo check -p indexer
cargo test -p indexer
./scripts/test-truenorth-integration.sh indexer
```

The integration harness creates an isolated daemon and removes its temporary
chain afterward. Invalidation and reconsideration only target that test node.
The default harness invocation still runs wallet tests.

The regtest test verifies P2QRH scripts and amounts against `validateaddress`
and `gettxout`, restart, same-height branch replacement, removal and subsequent
reconfirmation of a payment, rollback of more than fifty blocks, node failure,
wrong-network rejection, executable catch-up, and equality with a clean rebuild.
SQLite tests cover same-block parent/child spends, all-or-nothing failed writes,
abrupt exits before and after commit, exclusive writer ownership, reader snapshot
consistency, malformed data, duplicate IDs, and invalid spends.

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
