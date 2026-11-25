# Test conventions

Use the same layout in every workspace crate.

## Unit tests

Put unit tests at the bottom of the implementation file in an inline module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_the_expected_behavior() {
        // Arrange realistic inputs, perform the operation, assert its result.
    }
}
```

Keep tests with the code they exercise: address tests in `address.rs`, decoding
tests in `block.rs`, and so on. When a module's unit tests grow past ~250 lines
and dwarf the implementation, extract the test body into a sibling
`<module>_tests.rs` file linked with `#[cfg(test)] #[path = "<module>_tests.rs"] mod tests;`
so the module path (`<module>::tests`) stays unchanged while the implementation
file remains easy to scan.

A private `src/test_support.rs` at the crate root, enabled with `#[cfg(test)]`,
can hold fixtures shared by several implementation modules. Unit tests may use
temporary SQLite databases; they require no running daemon or user configuration.

Run all library unit tests:

```sh
cargo test --workspace --lib
```

To select a crate or module:

```sh
cargo test -p indexer --lib
cargo test -p indexer database::tests
```

`--lib` is optional for the usual `cargo test --workspace` run. It limits the run
to library test modules. The full command also builds binaries and integration
test targets, while skipping ignored daemon tests.

## Daemon integration tests

Put daemon scenarios in `<crate>/tests/truenorth_regtest.rs`. Put infrastructure
helpers in `<crate>/tests/support/mod.rs`, with larger helpers in submodules
under `support/`. Keep `support/` strictly for helpers; keep test scenarios
and assertions in `truenorth_regtest.rs`. Import helpers with `mod support;`;
avoid custom `#[path]` layouts in `tests/`.

Write focused, order-independent `#[test]` / `#[tokio::test]` functions that
work against any current chain height so individual scenarios can be filtered
and debugged independently. Mark each daemon test ignored with the harness
command:

```rust,ignore
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh indexer"]
```

The wallet, indexer, and server share this convention. Crates without a daemon
scenario do not need an empty integration-test target. The shared RPC client is
also exercised by the indexer and server scenarios.

Run the isolated harness for the whole workspace (default) or a single crate:

```sh
./scripts/test-truenorth-integration.sh
./scripts/test-truenorth-integration.sh wallet
./scripts/test-truenorth-integration.sh indexer
./scripts/test-truenorth-integration.sh server
```

`all`, `workspace`, and `--workspace` are also accepted explicitly to run all
workspace regtest suites (`wallet`, `indexer`, and `server`).

Pass an optional test filter to run a single matching scenario, and `-v` / `--verbose`
(or `VERBOSE=1`) to stream passing test output with `--nocapture`:

```sh
./scripts/test-truenorth-integration.sh server mining
./scripts/test-truenorth-integration.sh -v indexer
```

Each invocation starts a daemon with a unique temporary data directory, exports
its connection settings, and removes the chain afterward. The server command
runs the complete scenario twice automatically: first with daemon `txindex`
disabled, then enabled, on separate temporary chains. Both runs must pass;
no environment switch or second command is needed. CLI helpers require
`TRUENORTH_TEST_ISOLATED=1`; the marker is a guard, while the harness provides the
actual isolation. Never point mining or invalidation tests at a user's node.

The harness selects `--test truenorth_regtest -- --ignored --test-threads=1`
(adding `--nocapture` when `-v`/`--verbose` or `VERBOSE=1` is set): only daemon
tests run there, sequentially because they share the temporary funding wallet
and chain. No fixed sleep substitutes for readiness.

Test meaningful behavior with realistic data and checked integer amounts. Keep
blockchain scans such as `scantxoutset` in integration tests as correctness
oracles, rather than in production wallet request paths.

## API load testing with k6

Install [k6](https://grafana.com/docs/k6/latest/set-up/install-k6/) (`brew install k6`
on macOS). The script targets `https://api.northboundwallet.org`. Run:

```sh
./scripts/load-api.sh
```

The launcher enables the live dashboard and opens it automatically in your browser
at `http://localhost:5665`.

The test cycles through chain-height GET, fee-estimate GET, and transaction-history
POST requests at 50 requests/second total (about 16.7 per endpoint)
for ten minutes, with at most 30 requests in flight. The extra workers provide
headroom for connection setup and latency spikes without increasing the request
rate. It checks readiness before and
after the load. During the load, HTTP 503 responses count as failed requests but
do not stop the test. Other non-200 responses (including 429), timeouts, or invalid
response data abort the test. History requests query all three addresses hardcoded in the script
together, requesting the first page of up to 100 transactions without following
pagination. It fails if any endpoint's p95 exceeds one second or the generator
drops iterations because its concurrency cap is too low. These are client-side
measurements, including network latency; dropped iterations indicate the requested
rate was not sustained. This test exercises indexed history queries but does not
sign or broadcast transactions.

To repeat the bounded production run while watching CloudWatch:

```sh
RATE=50 MAX_VUS=30 \
  ./scripts/load-api.sh --summary-export=/tmp/northbound-k6-summary.json
```

Press Ctrl-C to stop early. `RATE` and `MAX_VUS` override the defaults above.
The URL and ten-minute duration are hardcoded in `scripts/load-api.js`. The normal server
budget is 100 requests/second shared with all users; keep production load below it.
