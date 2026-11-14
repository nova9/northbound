# Security model

TrueNorth Wallet is self-custodial. Recovery phrases, derived private keys,
passwords, and transaction signing stay in the desktop wallet process. The
server receives public addresses and signed transactions only.

## Local protection

- Wallet secrets are encrypted with Argon2id and XChaCha20-Poly1305.
- Wallet files are written atomically with owner-only permissions on Unix.
- Recovery phrases, passwords, decrypted secrets, and retained encryption keys
  are zeroized when their containers are dropped.
- The wallet file records its network and cannot be opened by a build for a
  different network.

## Server boundary

- Remote wallet server URLs must use HTTPS. HTTP is accepted only for
  `localhost` and loopback IP addresses. This applies to saved settings and
  build-time defaults as well as newly entered URLs. API requests do not follow
  redirects.
- The API server defaults to `127.0.0.1:3000`. `TRUENORTH_SERVER_BIND` can
  explicitly select another IP address and port; loopback binding is not enforced.
- The API has no authentication and exposes specific wallet operations, not
  arbitrary node RPC methods.
- Request bodies, address batches, and signed transaction sizes are bounded.
- Inputs from the server are treated as untrusted. The wallet verifies UTXO
  ownership and transaction details before signing.
- The server checks that its configured network matches the connected node.
- The node RPC cookie is read locally and is never returned by the API.

## Remaining assumptions

- The operating system, wallet executable, and local display are trusted.
- A malicious server can withhold or delay blockchain data. It cannot sign or
  redirect a transaction because transaction construction and signing are
  local, but users should verify recipient, amount, and fee before broadcast.
- Recovery requires access to a server that can scan the complete chain.
- This project has not received an independent cryptographic or application
  security audit. Mainnet releases should be reproducibly built, dependency
  audited, and reviewed by an independent security engineer.

## Future verification direction

Northbound currently uses a server-backed lightweight-wallet model rather than
SPV. The server supplies UTXOs, history, fee estimates, chain status, and
transaction propagation information.

The planned Electrum-style verification phase will download and validate
TrueNorth block headers locally and verify Merkle inclusion proofs for
confirmed transactions. The wallet will also support multiple servers, compare
fee and header information, and allow users to choose their own server.

These changes will reduce server trust but cannot remove it completely. A
server may still omit relevant transactions, unconfirmed transactions have no
block inclusion proof, and querying wallet addresses reveals information about
the wallet to the server. See [ROADMAP.md](ROADMAP.md) for the staged plan.

Report suspected vulnerabilities privately to the project maintainers. Do not
include recovery phrases, private keys, passwords, or live RPC credentials.
