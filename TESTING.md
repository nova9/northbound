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
tests in `block.rs`, database tests in `database.rs`, and so on. Do not create a
separate module file for each unit-test group. A private `src/test_support.rs`,
enabled with `#[cfg(test)]`, can hold fixtures shared by several implementation
modules. Unit tests may use temporary SQLite databases; they require no running
daemon or user configuration.

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
under `support/`. Import them with `mod support;`; avoid custom `#[path]` layouts.
Scenario-specific assertions can stay beside the scenario.

Use `#[test]` for synchronous tests and `#[tokio::test]` when the test awaits Rust
RPC or HTTP calls. Mark each daemon test ignored with the harness command:

```rust,ignore
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh indexer"]
```

The wallet, indexer, and server share this convention. Crates without a daemon
scenario do not need an empty integration-test target. The shared RPC client is
also exercised by the indexer and server scenarios.

Run the corresponding isolated harness:

```sh
./scripts/test-truenorth-integration.sh wallet
./scripts/test-truenorth-integration.sh indexer
./scripts/test-truenorth-integration.sh server
```

Each invocation starts a daemon with a unique temporary data directory, exports
its connection settings, and removes the chain afterward. CLI helpers require
`TRUENORTH_TEST_ISOLATED=1`; the marker is a guard, while the harness provides the
actual isolation. Never point mining or invalidation tests at a user's node.

The harness selects `--test truenorth_regtest -- --ignored --nocapture
--test-threads=1`: only daemon tests run there, sequentially because they share
the temporary funding wallet and chain. No fixed sleep substitutes for readiness.

Test meaningful behavior with realistic data and checked integer amounts. Keep
blockchain scans such as `scantxoutset` in integration tests as correctness
oracles, rather than in production wallet request paths.
