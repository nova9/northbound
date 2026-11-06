mod api;
mod config;
mod keys;
mod local_wallet;
mod p2qrh;
mod transaction_plan;
mod transaction_signing;
mod ui;
mod unsigned_transaction;
mod wallet_file;
mod wallet_secrets;

use std::str::FromStr;

use bitcoin::consensus::encode;
use bitcoin::{Amount, OutPoint, ScriptBuf, Txid};

pub use ui::App;

pub fn derive_receiving_address_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<String, String> {
    let public_key = keys::derive_receiving_public_key_from_entropy(entropy, address_index)?;

    p2qrh::address_from_public_key(&public_key, config::WALLET_NETWORK)
}

pub fn create_signed_payment_from_receiving_utxo(
    entropy: &[u8; 16],
    transaction_id: &str,
    output_index: u32,
    input_amount: u64,
    input_script_pubkey: &[u8],
    receiving_index: u32,
    recipient_address: &str,
    recipient_amount: u64,
    fee_rate: u64,
) -> Result<String, String> {
    let transaction_id = Txid::from_str(transaction_id)
        .map_err(|_| "funding transaction ID is invalid".to_owned())?;
    let available_utxo = local_wallet::VerifiedUtxo {
        outpoint: OutPoint::new(transaction_id, output_index),
        amount: Amount::from_sat(input_amount),
        script_pubkey: ScriptBuf::from_bytes(input_script_pubkey.to_vec()),
        keychain: local_wallet::WalletKeychain::Receiving,
        derivation_index: receiving_index,
        confirmations: 1,
        coinbase: false,
    };
    let change_address = derive_change_address_from_entropy(entropy, 0)?;
    let plan = transaction_plan::create_transaction_plan(
        vec![available_utxo],
        recipient_amount,
        fee_rate,
        change_address,
    )?;
    let mut transaction =
        unsigned_transaction::build_unsigned_transaction(&plan, recipient_address)?;

    transaction_signing::sign_p2qrh_transaction(&mut transaction, &plan.inputs, entropy)?;

    Ok(encode::serialize_hex(&transaction))
}

fn derive_change_address_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<String, String> {
    let public_key = keys::derive_change_public_key_from_entropy(entropy, address_index)?;

    p2qrh::address_from_public_key(&public_key, config::WALLET_NETWORK)
}
