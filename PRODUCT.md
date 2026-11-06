# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

## Users

Northbound is for people who want to use TrueNorth without giving a custodian
control of their funds or running a full node. The wallet should not require
knowledge of blockchain internals, descriptor syntax, key derivation, or node
operation.

## Product Purpose

Northbound is a lightweight desktop wallet for receiving, holding, and sending
NORTH. It should make self-custody understandable while keeping recovery words,
private keys, and transaction signing on the user's device.

Success means a person can create or restore a wallet, protect its recovery
words, understand the transaction they are approving, and send or receive
NORTH without operating `truenorthd` themselves.

## Positioning

Northbound is a community-developed wallet for TrueNorth. It combines a small
desktop application with local key ownership and a separate TrueNorth-aware
server for public blockchain data.

It complements a full-node wallet: Northbound prioritizes a lightweight user
experience, while a full-node wallet independently validates the complete
chain.

## Operating Context

The desktop application is written in Rust with Dioxus. Users create or restore
one wallet locally, unlock it with a local password, and use twelve recovery
words as the ultimate backup.

The wallet communicates with a Rust and Axum API backed by `truenorthd`. The
server receives public addresses and signed transactions, but never wallet
passwords, recovery words, seeds, or private keys.

Local development uses TrueNorth regtest. The complete wallet flow has also
been tested successfully against TrueNorth mainnet. There is not yet a public
server or downloadable release.

## Current Capabilities

- Create and restore a wallet from twelve English BIP39 recovery words.
- Store wallet secrets in a versioned, password-encrypted local file.
- Unlock the wallet and change its local password.
- Derive native P2QRH receiving and change addresses locally.
- Discover address activity and recover used derivation indexes.
- Display balances, transaction history, and confirmation status.
- Select UTXOs and calculate an explicit transaction plan.
- Show the recipient, amount, and fee before signing.
- Sign P2QRH transactions locally and broadcast the signed transaction.
- Prevent allocation beyond nineteen consecutive unused receiving addresses.

## Constraints and Trust Boundary

- The current version supports one wallet and one account.
- Regtest and mainnet builds are supported; Testnet4 is future work.
- The configured server supplies UTXOs, history, fee estimates, chain status,
  transaction validation, and broadcasting.
- A dishonest server cannot sign or redirect a reviewed transaction, but it
  can omit or delay information, suggest a poor fee, or refuse to broadcast.
- Future Electrum-style verification will validate TrueNorth block headers and
  confirmed-transaction inclusion proofs locally.
- P2QRH behavior must continue to be tested against TrueNorth Core rather than
  assumed compatible with ordinary Bitcoin libraries.
- The interface must never silently change recipients, amounts, fees, or
  wallet state.

## Brand Commitments

Northbound has its own product name and visual identity while remaining clear
about its purpose as a TrueNorth wallet. Its logo and design tokens are
centralized so the identity can evolve without restructuring the interface.

The voice is direct, calm, and understandable to people who are not blockchain
specialists. It states risks plainly without hype or unnecessary alarm.

## Evidence on Hand

- P2QRH address vectors have been checked against TrueNorth Core regtest.
- Automated tests cover key derivation, encrypted storage, recovery, UTXO
  selection, transaction planning, signing, and repeated spending.
- The wallet has completed a successful TrueNorth mainnet test.
- The project has no independent security audit, public release artifacts,
  user testimonials, or production-readiness claim.

## Product Principles

1. Make self-custody understandable without hiding its consequences.
2. Keep secrets local and make sensitive operations explicit.
3. Show the user one clear next action at a time.
4. Prefer calm, trustworthy language over cryptocurrency jargon or hype.
5. Make server trust and connection state visible.
6. Verify TrueNorth-specific behavior against its authoritative implementation.

## Accessibility and Inclusion

Important status and error information must not depend on color alone.
Controls need visible labels, keyboard focus must remain clear, and recovery and
transaction flows must be understandable without specialist terminology.
