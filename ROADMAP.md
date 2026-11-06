# Roadmap

Northbound is being developed one complete wallet operation at a time. The
immediate goal is a small, understandable wallet whose transaction details and
security boundaries remain visible to the user.

This roadmap describes direction rather than promised release dates.

## Release readiness

- Verify address derivation, recovery, transaction construction, and P2QRH
  signing against TrueNorth Core on regtest.
- Add Windows and Linux builds and test the interface on each platform.
- Publish signed artifacts with checksums and build provenance.
- Document operation of the public Northbound server.
- Arrange independent review before recommending use with meaningful funds.

## Server choice and resilience

- Let users configure their own Northbound server.
- Support multiple independent servers and compare important responses.
- Apply local lower and upper safety limits to fee estimates.
- Retry transaction broadcast through another configured server when the first
  server is unavailable.
- Make server identity and connection state visible in the wallet.

## Electrum-style verification

The current wallet trusts its server to report blockchain data. A future phase
will reduce that trust using an approach similar to Electrum:

1. Download TrueNorth block headers to the wallet.
2. Validate header linkage, RandomNorth proof of work, and TrueNorth difficulty
   rules locally.
3. Add server endpoints that return Merkle inclusion proofs for confirmed
   transactions.
4. Verify each confirmed wallet transaction against the locally validated
   header chain.
5. Derive the wallet balance from verified transaction history instead of
   accepting a server-provided balance as chain truth.
6. Compare header tips and fee estimates from multiple servers.

This will prevent a server from inventing a confirmed transaction. It will not
completely prevent a server from hiding a transaction, and unconfirmed
transactions will continue to require additional care because they have no
block inclusion proof.

TrueNorth-specific consensus behavior must be implemented and tested against
TrueNorth Core; Bitcoin SPV code cannot be assumed compatible without that
verification.

## Privacy

- Explain that address queries allow a server to associate addresses with one
  wallet and observe the client's network address.
- Support user-operated servers as the strongest initial privacy option.
- Investigate proxy and Tor support without hiding its operational tradeoffs.
- Avoid accounts, analytics, and unnecessary identifying information.

## Later wallet capabilities

- Testnet4 support.
- Payment URI and QR-code requests once the TrueNorth URI format is agreed.
- Watch-only wallets.
- Address labels and transaction notes stored locally.
- Hardware-wallet research for P2QRH.
