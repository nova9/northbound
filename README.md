# Northbound

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
scripts/  Local development and packaging helpers
```

## Local development

Install a Rust toolchain, then download a TrueNorth release and start a local
regtest node:

```bash
./scripts/download-truenorth.sh \
  https://github.com/truenorth-project/truenorth/releases/download/v30.5.2/truenorth-v30.5.2-macos-arm64.tar.gz
./scripts/regtest-start.sh
```

Run the server and wallet in separate terminals:

```bash
cargo run -p server
cargo run -p wallet
```

Run the automated tests:

```bash
cargo test --workspace
```

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
