use std::env;
use std::process::Command;

use serde_json::Value;
use wallet::{create_signed_payment_from_receiving_utxo, derive_receiving_address_from_entropy};

#[test]
#[ignore = "requires TrueNorth Core regtest"]
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

    assert!(receiving_address.starts_with("bcrt1z"));
    println!("Northbound receiving address: {receiving_address}");

    let address_argument = format!("address={receiving_address}");
    let funding_wallet = env::var("TRUENORTH_TEST_WALLET")
        .expect("TRUENORTH_TEST_WALLET must name the funding wallet");
    let wallet_argument = format!("-rpcwallet={funding_wallet}");

    // Core creates and signs the transaction that funds Northbound with 1 NORTH.
    let transaction_id = run_truenorth_cli(&[
        &wallet_argument,
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
    let funding_transaction = run_funding_wallet_cli(&["gettransaction", &transaction_id, "true"]);
    let funding_transaction: Value = serde_json::from_str(&funding_transaction)
        .expect("funding transaction should be valid JSON");
    let funding_transaction_hex = funding_transaction["hex"]
        .as_str()
        .expect("funding transaction should contain raw transaction hex");
    let decoded_transaction = run_truenorth_cli(&["decoderawtransaction", funding_transaction_hex]);
    let decoded_transaction: Value = serde_json::from_str(&decoded_transaction)
        .expect("decoded funding transaction should be valid JSON");
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
        1,
    )
    .expect("Northbound should sign a payment back to the funding wallet");

    // Ask Core whether consensus and mempool rules accept Northbound's signed
    // transaction before broadcasting it.
    let transactions_to_validate = format!(r#"["{signed_transaction}"]"#);
    let validation = run_truenorth_cli(&["testmempoolaccept", &transactions_to_validate]);
    let validation: Value =
        serde_json::from_str(&validation).expect("validation result should be valid JSON");
    assert_eq!(
        validation[0]["allowed"], true,
        "Core rejected the transaction"
    );

    // Broadcast the Northbound transaction and mine one block to confirm it.
    let return_transaction_id = run_truenorth_cli(&["sendrawtransaction", &signed_transaction]);
    run_funding_wallet_cli(&["generatetoaddress", "1", &mining_address]);

    // Confirm Core's funding wallet recognized the returned payment.
    let returned_payment = run_funding_wallet_cli(&["gettransaction", &return_transaction_id]);
    let returned_payment: Value =
        serde_json::from_str(&returned_payment).expect("returned payment should be valid JSON");
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
    let decoded_return_transaction =
        run_truenorth_cli(&["decoderawtransaction", &signed_transaction]);
    let decoded_return_transaction: Value = serde_json::from_str(&decoded_return_transaction)
        .expect("decoded return transaction should be valid JSON");
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

fn print_balances(step: &str, core_balance: &str, northbound_balance: &str) {
    println!("{step}:");
    println!("  Core funding wallet: {core_balance} NORTH");
    println!("  Northbound wallet: {northbound_balance} NORTH");
}

fn run_funding_wallet_cli(arguments: &[&str]) -> String {
    let funding_wallet = env::var("TRUENORTH_TEST_WALLET")
        .expect("TRUENORTH_TEST_WALLET must name the funding wallet");
    let wallet_argument = format!("-rpcwallet={funding_wallet}");
    let mut wallet_arguments = vec![wallet_argument.as_str()];
    wallet_arguments.extend_from_slice(arguments);

    run_truenorth_cli(&wallet_arguments)
}

fn run_truenorth_cli(arguments: &[&str]) -> String {
    let cli_path =
        env::var("TRUENORTH_TEST_CLI").expect("TRUENORTH_TEST_CLI must point to truenorth-cli");
    let data_directory = env::var("TRUENORTH_TEST_DATA_DIR")
        .expect("TRUENORTH_TEST_DATA_DIR must point to the test node data");
    let rpc_port =
        env::var("TRUENORTH_TEST_RPC_PORT").expect("TRUENORTH_TEST_RPC_PORT must be set");

    let output = Command::new(cli_path)
        .arg("-regtest")
        .arg(format!("-datadir={data_directory}"))
        .arg(format!("-rpcport={rpc_port}"))
        .args(arguments)
        .output()
        .expect("truenorth-cli should start");

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        panic!("truenorth-cli failed: {error}");
    }

    String::from_utf8(output.stdout)
        .expect("truenorth-cli output should be UTF-8")
        .trim()
        .to_owned()
}
