use std::{env, process::Command};

pub fn print_balances(step: &str, core_balance: &str, northbound_balance: &str) {
    println!("{step}:");
    println!("  Core funding wallet: {core_balance} NORTH");
    println!("  Northbound wallet: {northbound_balance} NORTH");
}

pub fn run_funding_wallet_cli(arguments: &[&str]) -> String {
    let funding_wallet = env::var("TRUENORTH_TEST_WALLET")
        .expect("TRUENORTH_TEST_WALLET must name the funding wallet");
    let wallet_argument = format!("-rpcwallet={funding_wallet}");
    let mut wallet_arguments = vec![wallet_argument.as_str()];
    wallet_arguments.extend_from_slice(arguments);

    run_cli(&wallet_arguments)
}

pub fn run_cli(arguments: &[&str]) -> String {
    assert_eq!(
        env::var("TRUENORTH_TEST_ISOLATED").as_deref(),
        Ok("1"),
        "run scripts/test-truenorth-integration.sh wallet; tests require an isolated node"
    );
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
