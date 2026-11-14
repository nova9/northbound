# P2QRH compatibility tests

Run `cargo test -p wallet --lib` for the local unit tests. They stay alongside
the code in `wallet/src/p2qrh.rs` and `wallet/src/transaction_signing.rs`.
`cargo test --workspace` also runs these tests without starting a node.

Run `./scripts/test-truenorth-integration.sh` for the two ignored tests in
`truenorth_regtest.rs`, against the Core release in the repository's
[`TRUENORTH_VERSION`](../../../TRUENORTH_VERSION). The script
runs them sequentially because they share a node and funding wallet. One test
funds, spends, and confirms a payment. The other independently funds a spend,
checks its descriptor-derived address, and tests mutated witnesses.

The reference is the 18 `qrh_*` cases in Core's
[`src/test/truenorth_tests.cpp`](https://github.com/truenorth-project/truenorth/blob/f58d269d10f1c80ab80b31092dd96a4f9436fe3c/src/test/truenorth_tests.cpp).
Northbound uses the actual node to validate spends rather than implementing a
second consensus interpreter in its tests.

This is a behavior mapping, not a port of all 18 Core unit tests. Local unit
tests cover commitment construction, witness layout, signature verification,
signing refusal, and preservation of reviewed transaction details. Only the
node-backed integration tests establish Core's acceptance or rejection.

| Core case (`qrh_` prefix omitted) | Northbound coverage |
| --- | --- |
| `commitment_helper_deterministic` | `p2qrh::tests::commitment_is_deterministic` |
| `commitment_helper_differs_by_scheme_id` | `p2qrh::tests::commitment_changes_with_scheme_id` |
| `commitment_helper_differs_by_pubkey` | `p2qrh::tests::commitment_changes_with_public_key` |
| `commitment_helper_differs_by_script_root` | `p2qrh::tests::commitment_changes_with_script_root` |
| `valid_key_path_spend_accepts` | Regtest checks `testmempoolaccept`, broadcasts, mines, and verifies receipt. |
| `unknown_scheme_id_rejects` | Regtest replaces the witness scheme with `02`. |
| `scheme_id_zero_rejects` | Regtest replaces the witness scheme with `00`. |
| `scheme_id_wrong_byte_length_rejects` | Regtest replaces the witness scheme with `0100`. |
| `wrong_pubkey_size_rejects` | Regtest truncates the revealed public key to 31 bytes. |
| `commitment_mismatch_rejects` | Regtest flips one bit of the revealed public key. |
| `invalid_signature_rejects` | Regtest corrupts the signature. |
| `empty_witness_rejects` | Regtest clears the witness. |
| `too_few_stack_items_rejects` | Regtest removes the scheme item. |
| `extra_stack_items_rejects` | Regtest prepends an unexpected item. |
| `flag_off_passes_through` | Core-only: RPC does not expose `SCRIPT_VERIFY_QRH` flags. |
| `descriptor_populates_spend_data` | Partial integration coverage: regtest checks Core's `deriveaddresses` for `qrh(revealed_public_key)` against Northbound's funded address. Core's internal provider data is not exposed. |
| `sign_step_produces_valid_witness` | Local signature verification and regtest acceptance of Northbound's signature. |
| `sign_step_without_spend_data_fails` | Northbound has no Core signing provider. Its local `wallet_refuses_to_sign_with_the_wrong_derivation_index` test checks rejection when the derived key does not match the UTXO. This is a related wallet safeguard, not the same Core case. |

Every malformed witness starts from the same valid signed transaction and
unspent output. The test requires the expected script error, so an unrelated
fee, missing-input, or conflict rejection cannot satisfy the assertion. These
checks do not broadcast the candidate transaction; the separate round-trip
test covers broadcasting and confirmation.

Additional local tests check that signing refuses another wallet's entropy,
the wrong keychain, a mismatched commitment, missing UTXO data, and a different
outpoint. They also check that signing changes only the witness and that the
signature commits to the recipient, output amounts, fee, and previous output.
Both receiving-input signatures and the change-input signature are verified
in the existing multi-input and repeated-spend unit test.

These tests cover Schnorr key-path spends. They do not establish script-path
support or replace an independent security review.
