<img src="wallet/assets/logo.svg" alt="Northbound logo" width="96" height="96">

# Northbound

[northboundwallet.org](https://northboundwallet.org)

Northbound is an experimental, self-custodial lightweight desktop wallet for
[TrueNorth](https://github.com/truenorth-project/truenorth).

The desktop application is built with Rust and Dioxus. It creates addresses,
stores encrypted wallet secrets, and signs transactions on the user's device.
A separate Rust and Axum API reads public blockchain data from `truenorthd` and
broadcasts transactions that have already been signed by the wallet.

Northbound is an independent community project. It is not an official
TrueNorth wallet and has not received an independent security audit. Do not use
it with significant funds.

## Current capabilities

- Create or restore a wallet with 12 recovery words.
- Keep multiple named wallets on one device, each with its own password.
- Encrypt the local wallet file with Argon2id and XChaCha20-Poly1305.
- Derive native TrueNorth P2QRH receiving and change addresses locally.
- Display balances and transaction history supplied by the wallet server.
- Construct and review the recipient, amount, and fee before signing.
- Sign P2QRH transactions locally and broadcast the signed transaction.
- Change the local wallet password without changing the recovery phrase.

## Security and server trust

Recovery words, private keys, passwords, and transaction signing remain in the
desktop wallet. They are never sent to the Northbound server.

The current version relies on its configured server for address activity,
UTXOs, transaction history, fee estimates, chain height, transaction status,
validation, and broadcasting. A dishonest or unavailable server cannot sign a
transaction, but it can hide or delay blockchain information, provide
misleading history, suggest a poor fee, or refuse to broadcast.

See [SECURITY.md](SECURITY.md) for the current security boundary and
[ROADMAP.md](ROADMAP.md) for the planned move toward Electrum-style local
verification.

## Repository structure

```text
wallet/   Dioxus desktop wallet and local signing
server/   Axum API backed by truenorthd
shared/   JSON request and response types
indexer/  SQLite chain index, pending snapshots, and wallet queries
truenorth-rpc/ Shared daemon RPC transport and decoding
scripts/  Local development and packaging helpers
```

## Local development

Use **Wallets** in the header to create, restore, switch, or remove wallets.
Switching locks the current wallet. **Open wallet folder** opens the storage
folder in Finder, Windows Explorer, or the Linux file manager.

Each wallet is an encrypted `<name>.json` file. Development wallets live in
`.local/wallets/<network>/`; release wallets use the operating system's local
data folder under `io.github.nova9.northbound/wallets/<network>/`.
The previous single `wallet.json` is not imported automatically; restore that
wallet with its recovery words. Removing a wallet permanently deletes its local
file after a backup confirmation and an exact name check. It does not move funds
or remove other copies of the file.

Install a Rust toolchain, then download a TrueNorth release and start a local
regtest node:

```bash
./scripts/download-truenorth.sh
./scripts/regtest.sh start
```

Create and fund the development wallet, send coins, or stop the node:

```bash
./scripts/regtest.sh create-wallet funding
./scripts/regtest.sh get-address funding
./scripts/regtest.sh generate-coins funding 101
./scripts/regtest.sh send-coins funding <receiving-address> [amount]
./scripts/regtest.sh stop
```

Mining takes a Core wallet name and defaults to 101 blocks; sending
defaults to 10 NORTH. `get-address` generates a fresh receiving address for the
named wallet. Address, mining, and sending commands automatically load an
existing wallet when needed. Development
commands share the persistent data directory `.local/truenorth/`.

Run the server and wallet in separate terminals:

```bash
cargo run -p server
cargo run -p wallet
```

Run the automated tests:

```bash
cargo test --workspace
```

Run the P2QRH checks against an isolated TrueNorth Core regtest node:

```bash
./scripts/test-truenorth-integration.sh
```

Development downloads and integration tests use the Core release pinned in
[`TRUENORTH_VERSION`](TRUENORTH_VERSION). All local regtest scripts use the shared
binaries in `.local/truenorth-bin/<version>/<platform>/`. The integration script
calls the downloader, reuses cached releases, and removes its temporary chain
on exit. Downloads are verified against the release's SHA-256 checksum.

To download another release without changing the project's selected version:

```bash
./scripts/download-truenorth.sh 30.5.2
```

Versions with a `v` prefix are also accepted. Update `TRUENORTH_VERSION` to select
which release the development and integration scripts use. Deployment versions
under `server/devops/` are managed independently.
See [test conventions and commands](TESTING.md) for the shared unit-test layout
and isolated wallet, indexer, and server regtest suites.

The [server](server/README.md) starts the [SQLite indexer](indexer/README.md),
follows reorganizations, and serves wallet data through indexed queries. It uses
`.local/indexer/chain.sqlite` by default; set `TRUENORTH_INDEX_DATABASE` to change
the path. Stop any standalone indexer writing that file before running the server.
`GET /ready` reports catch-up and freshness; wallet data returns a retryable error
until confirmed and pending snapshots are ready. Fee estimation, validation, and
broadcast remain connected directly to the daemon.

Run the complete indexed API checks with an isolated node:

```sh
./scripts/test-truenorth-integration.sh server
```

Build the mainnet desktop wallet for the host OS:

```bash
./scripts/build-wallet.sh        # Linux AppImage or macOS app
./scripts/build-windows.ps1      # Windows MSI, run in PowerShell
```

Publishing a GitHub release, including a pre-release such as `v0.1.0-rc.1`,
runs the macOS release workflow. Commit the workflow before creating the tag.
It builds the mainnet wallet for Apple Silicon and attaches an ad-hoc signed
`Northbound-<tag>-macos-arm64.zip` app archive and SHA-256 checksum to the release
(for example, `Northbound-v0.1.0-rc.1-macos-arm64.zip`).
Draft releases do not trigger builds. To build without publishing a release,
use **Actions → macOS release build → Run workflow**; download the archive
from that run's artifacts. Manual builds use `dev-<short-commit>` in the filename.
macOS builds do not yet have an Apple Developer ID
signature or notarization.

Build and publish Docker images after setting `DOCKERHUB_REPOSITORY`:

```bash
./scripts/build-image.sh server [version]
./scripts/build-image.sh truenorthd
```

The server version defaults to its Cargo package version, including workspace
inheritance; the daemon version comes from `server/devops/truenorthd/VERSION.txt`.
Both commands push the built image.
`APP_SLUG` and `DOCKER_PLATFORMS` remain available as environment overrides.

Mainnet builds use the local server by default. Set the server URL explicitly
when building a wallet that should connect to a hosted server:

```bash
NORTHBOUND_API_BASE_URL="https://wallet-api.example.com" \
  cargo build -p wallet --release --no-default-features --features mainnet
```

## Project status

Northbound is under active development. Before a general release, the project
still needs broader TrueNorth regtest coverage, Windows and Linux packaging,
signed release artifacts, independent review, and a clearly documented public
server deployment.

Bug reports and technical review are welcome. Please report security problems
privately and never include recovery words, private keys, passwords, or live
RPC credentials.

## License

Northbound is available under the [MIT License](LICENSE), the same license used
by TrueNorth Core.
