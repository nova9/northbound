mod amount;
mod config;
mod crypto;
mod server;
mod storage;
mod transactions;
mod ui;
mod updates;
mod wallet;

use std::str::FromStr;

use bitcoin::{Amount, OutPoint, ScriptBuf, Txid};

pub use ui::App;

pub fn derive_receiving_address_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<String, String> {
    let public_key =
        crypto::keys::derive_receiving_public_key_from_entropy(entropy, address_index)?;

    crypto::p2qrh::address_from_public_key(&public_key, config::WALLET_NETWORK)
}

#[allow(clippy::too_many_arguments)]
pub fn create_signed_payment_from_receiving_utxo(
    entropy: &[u8; 16],
    transaction_id: &str,
    output_index: u32,
    input_amount: u64,
    input_script_pubkey: &[u8],
    receiving_index: u32,
    recipient_address: &str,
    recipient_amount: u64,
    atomic_units_per_kvbyte: u64,
) -> Result<String, String> {
    let transaction_id = Txid::from_str(transaction_id)
        .map_err(|_| "funding transaction ID is invalid".to_owned())?;
    let available_utxo = transactions::VerifiedUtxo {
        outpoint: OutPoint::new(transaction_id, output_index),
        amount: Amount::from_sat(input_amount),
        script_pubkey: ScriptBuf::from_bytes(input_script_pubkey.to_vec()),
        keychain: transactions::WalletKeychain::Receiving,
        derivation_index: receiving_index,
        confirmations: 1,
        coinbase: false,
    };
    let change_address = derive_change_address_from_entropy(entropy, 0)?;
    let plan = transactions::create_transaction_plan(
        vec![available_utxo],
        recipient_amount,
        atomic_units_per_kvbyte,
        change_address,
    )?;
    let signed = transactions::build_and_sign_p2qrh_transaction(&plan, recipient_address, entropy)?;

    Ok(signed.transaction_hex)
}

fn derive_change_address_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<String, String> {
    let public_key = crypto::keys::derive_change_public_key_from_entropy(entropy, address_index)?;

    crypto::p2qrh::address_from_public_key(&public_key, config::WALLET_NETWORK)
}
