mod support;

use bitcoin::consensus::{deserialize, serialize};
use bitcoin::{Transaction, Witness};
use support::{cli_json, funding_wallet_cli_json, print_balances, run_cli, run_funding_wallet_cli};
use wallet::{create_signed_payment_from_receiving_utxo, derive_receiving_address_from_entropy};

#[test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh wallet"]
fn northbound_receives_funds_from_core_and_sends_some_back() {
    let wallet_entropy = [0_u8; 16];

    // Coinbase rewards cannot be spent until they have 100 confirmations.
    // Mining 101 blocks gives Core's funding wallet one spendable reward.
    let mining_address = run_funding_wallet_cli(&["getnewaddress"]);
    run_funding_wallet_cli(&["generatetoaddress", "101", &mining_address]);
    let core_balance_after_mining = run_funding_wallet_cli(&["getbalance"]);
    print_balances("After mining", &core_balance_after_mining, "0.00000000");

    // Northbound derives this address locally from its own wallet entropy.
    let receiving_address = derive_receiving_address_from_entropy(&wallet_entropy, 0)
        .expect("first receiving address should derive");

    assert!(receiving_address.starts_with("rnorth1z"));
    println!("Northbound receiving address: {receiving_address}");

    // Core creates and signs the transaction that funds Northbound with 1 NORTH.
    let address_argument = format!("address={receiving_address}");
    let transaction_id = run_funding_wallet_cli(&[
        "-named",
        "sendtoaddress",
        &address_argument,
        "amount=1",
        "fee_rate=1",
    ]);

    assert_eq!(transaction_id.len(), 64);
    assert!(
        transaction_id
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
    println!("Funding transaction: {transaction_id}");

    // Confirm the funding transaction so Northbound can spend its output.
    run_funding_wallet_cli(&["generatetoaddress", "1", &mining_address]);
    let core_balance_after_funding = run_funding_wallet_cli(&["getbalance"]);
    print_balances(
        "After funding Northbound",
        &core_balance_after_funding,
        "1.00000000",
    );

    // Find the exact transaction output that belongs to Northbound. Its index
    // and locking script are required when Northbound signs the return spend.
    let funding_transaction = funding_wallet_cli_json(&["gettransaction", &transaction_id, "true"]);
    let funding_transaction_hex = funding_transaction["hex"]
        .as_str()
        .expect("funding transaction should contain raw transaction hex");
    let decoded_transaction = cli_json(&["decoderawtransaction", funding_transaction_hex]);
    let funded_output = decoded_transaction["vout"]
        .as_array()
        .expect("funding transaction should contain outputs")
        .iter()
        .find(|output| output["scriptPubKey"]["address"] == receiving_address)
        .expect("funding transaction should pay the Northbound address");
    let output_index = funded_output["n"]
        .as_u64()
        .expect("funded output should have an index") as u32;
    let script_pubkey = funded_output["scriptPubKey"]["hex"]
        .as_str()
        .expect("funded output should contain its script");
    let script_pubkey = hex::decode(script_pubkey).expect("funded output script should be hex");

    // Northbound builds and signs a transaction returning 0.5 NORTH to Core.
    // Any remaining value becomes Northbound change after paying the fee.
    let return_address = run_funding_wallet_cli(&["getnewaddress"]);
    let signed_transaction = create_signed_payment_from_receiving_utxo(
        &wallet_entropy,
        &transaction_id,
        output_index,
        100_000_000,
        &script_pubkey,
        0,
        &return_address,
        50_000_000,
        1_001,
    )
    .expect("Northbound should sign a payment back to the funding wallet");

    let transaction: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&hex::decode(&signed_transaction).unwrap()).unwrap();
    let mut output_total = 0;
    for output in &transaction.output {
        output_total += output.value.to_sat();
    }
    assert_eq!(100_000_000 - output_total, 165);

    // Ask Core whether consensus and mempool rules accept Northbound's signed
    // transaction before broadcasting it.
    let transactions_to_validate = format!(r#"["{signed_transaction}"]"#);
    let validation = cli_json(&["testmempoolaccept", &transactions_to_validate]);
    assert_eq!(
        validation[0]["allowed"], true,
        "Core rejected the transaction"
    );

    // Broadcast the Northbound transaction and mine one block to confirm it.
    let return_transaction_id = run_cli(&["sendrawtransaction", &signed_transaction]);
    run_funding_wallet_cli(&["generatetoaddress", "1", &mining_address]);

    // Confirm Core's funding wallet recognized the returned payment.
    let returned_payment = funding_wallet_cli_json(&["gettransaction", &return_transaction_id]);
    assert_eq!(returned_payment["confirmations"].as_u64(), Some(1));
    assert!(
        returned_payment["details"]
            .as_array()
            .expect("returned payment should contain wallet details")
            .iter()
            .any(|detail| detail["address"] == return_address && detail["category"] == "receive"),
        "funding wallet did not receive the returned payment"
    );
    println!("Returned transaction: {return_transaction_id}");

    // Core reports its own wallet balance. Northbound's remaining balance is
    // the change output from the transaction it created and signed.
    let core_balance = run_funding_wallet_cli(&["getbalance"]);
    let decoded_return_transaction = cli_json(&["decoderawtransaction", &signed_transaction]);
    let northbound_change = decoded_return_transaction["vout"]
        .as_array()
        .expect("return transaction should contain outputs")
        .iter()
        .find(|output| output["scriptPubKey"]["address"] != return_address)
        .map(|output| &output["value"])
        .expect("return transaction should contain Northbound change");

    print_balances(
        "After Northbound returns funds",
        &core_balance,
        &northbound_change.to_string(),
    );
}

#[test]
#[ignore = "requires isolated TrueNorth regtest; run scripts/test-truenorth-integration.sh wallet"]
fn core_validates_northbound_p2qrh_witnesses_and_descriptor() {
    let entropy = [0_u8; 16];
    let receiving_address = derive_receiving_address_from_entropy(&entropy, 1)
        .expect("receiving address should derive");

    // Mine mature funds for this check independently of the round-trip test.
    let mining_address = run_funding_wallet_cli(&["getnewaddress"]);
    run_funding_wallet_cli(&["generatetoaddress", "101", &mining_address]);
    let transaction_id = run_funding_wallet_cli(&[
        "-named",
        "sendtoaddress",
        &format!("address={receiving_address}"),
        "amount=1",
        "fee_rate=1",
    ]);
    run_funding_wallet_cli(&["generatetoaddress", "1", &mining_address]);
    let funding = funding_wallet_cli_json(&["gettransaction", &transaction_id]);
    let funding = cli_json(&[
        "decoderawtransaction",
        funding["hex"].as_str().expect("funding should contain hex"),
    ]);
    let output = funding["vout"]
        .as_array()
        .expect("funding should contain outputs")
        .iter()
        .find(|output| output["scriptPubKey"]["address"] == receiving_address)
        .expect("funding should pay Northbound");
    let output_index = u32::try_from(output["n"].as_u64().expect("output should have an index"))
        .expect("output index should fit u32");
    let script = hex::decode(
        output["scriptPubKey"]["hex"]
            .as_str()
            .expect("output should have a script"),
    )
    .expect("output script should be hex");
    let recipient = run_funding_wallet_cli(&["getnewaddress"]);
    let signed_transaction = create_signed_payment_from_receiving_utxo(
        &entropy,
        &transaction_id,
        output_index,
        100_000_000,
        &script,
        1,
        &recipient,
        50_000_000,
        1_001,
    )
    .expect("Northbound should sign its funded output");

    // Establish acceptance before trying invalid witnesses on the same UTXO.
    let transactions =
        serde_json::to_string(&[&signed_transaction]).expect("transaction list should serialize");
    let result = cli_json(&["testmempoolaccept", &transactions]);
    assert_eq!(
        result[0]["allowed"], true,
        "Core rejected the valid spend: {result}"
    );

    check_core_descriptor_address(&signed_transaction, &receiving_address);
    check_core_rejects_malformed_witnesses(&signed_transaction);
}

// Mirrors the key-path consensus cases in Core's src/test/truenorth_tests.cpp.
// Use Core as the validator rather than duplicating its consensus interpreter.
fn check_core_rejects_malformed_witnesses(signed_transaction: &str) {
    let bytes = hex::decode(signed_transaction).expect("signed transaction should be hex");
    let transaction: Transaction = deserialize(&bytes).expect("transaction should decode");
    let valid_stack = transaction.input[0].witness.to_vec();
    assert_eq!(valid_stack.len(), 3);
    assert_eq!(valid_stack[0].len(), 64);
    assert_eq!(valid_stack[1].len(), 32);
    assert_eq!(valid_stack[2], [1]);

    let inactive_scheme = "P2QRH scheme id is not active";
    let wrong_element_count = "P2QRH key-path witness must have exactly three elements";
    let cases = [
        ("unknown_scheme_id", inactive_scheme),
        ("scheme_id_zero", inactive_scheme),
        (
            "scheme_id_wrong_byte_length",
            "P2QRH scheme id must be exactly one byte",
        ),
        (
            "wrong_pubkey_size",
            "P2QRH public key has the wrong size for its scheme",
        ),
        ("commitment_mismatch", "Witness program hash mismatch"),
        ("invalid_signature", "Invalid Schnorr signature"),
        (
            "empty_witness",
            "Witness program was passed an empty witness",
        ),
        ("too_few_stack_items", wrong_element_count),
        ("extra_stack_items", wrong_element_count),
    ];

    for (case, expected_error) in cases {
        let mut stack = valid_stack.clone();
        match case {
            "unknown_scheme_id" => stack[2] = vec![2],
            "scheme_id_zero" => stack[2] = vec![0],
            "scheme_id_wrong_byte_length" => stack[2] = vec![1, 0],
            "wrong_pubkey_size" => {
                stack[1].pop();
            }
            "commitment_mismatch" => stack[1][0] ^= 1,
            "invalid_signature" => stack[0][0] ^= 0xff,
            "empty_witness" => stack.clear(),
            "too_few_stack_items" => {
                // Keep the scheme ID last so Core reaches the element-count check.
                stack.remove(0);
            }
            "extra_stack_items" => stack.insert(0, vec![0xde, 0xad]),
            _ => unreachable!("all cases are listed above"),
        }

        let mut malformed_transaction = transaction.clone();
        malformed_transaction.input[0].witness = Witness::from_slice(&stack);
        let transactions =
            serde_json::to_string(&vec![hex::encode(serialize(&malformed_transaction))])
                .expect("transaction list should serialize");
        let result = cli_json(&["testmempoolaccept", &transactions]);
        assert_eq!(
            result[0]["allowed"], false,
            "Core accepted {case}: {result}"
        );
        let reason = result[0]["reject-reason"]
            .as_str()
            .expect("rejection should explain why");
        // Require the script error, so fee, UTXO, conflict, or policy failures
        // cannot make a malformed-witness test pass for the wrong reason.
        assert!(
            reason.contains("script-verify-flag") && reason.contains(expected_error),
            "unexpected rejection for {case}: {result}"
        );
        println!("Core rejected {case}: {reason}");
    }
}

fn check_core_descriptor_address(signed_transaction: &str, receiving_address: &str) {
    let bytes = hex::decode(signed_transaction).expect("signed transaction should be hex");
    let transaction: Transaction = deserialize(&bytes).expect("transaction should decode");
    let public_key = hex::encode(&transaction.input[0].witness.to_vec()[1]);
    let descriptor = format!("qrh({public_key})");
    let info = cli_json(&["getdescriptorinfo", &descriptor]);
    let descriptor = info["descriptor"]
        .as_str()
        .expect("Core should return a descriptor");
    let addresses = cli_json(&["deriveaddresses", descriptor]);
    assert_eq!(
        addresses[0], receiving_address,
        "Core and Northbound must derive the same P2QRH commitment and address"
    );
}
