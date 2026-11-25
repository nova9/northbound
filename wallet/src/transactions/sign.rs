use bitcoin::consensus::encode;
use bitcoin::secp256k1::{Keypair, Message, Secp256k1};
use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
use bitcoin::{Transaction, TxOut, Witness};

use crate::crypto::{keys, p2qrh};
use crate::transactions::build::build_unsigned_transaction;
use crate::transactions::plan::TransactionPlan;
use crate::transactions::utxo::{VerifiedUtxo, WalletKeychain};

const SCHNORR_SCHEME_ID: u8 = 0x01;

pub struct SignedTransaction {
    pub transaction_hex: String,
    pub transaction_id: String,
}

pub fn build_and_sign_p2qrh_transaction(
    plan: &TransactionPlan,
    recipient_address: &str,
    wallet_entropy: &[u8; 16],
) -> Result<SignedTransaction, String> {
    let mut transaction = build_unsigned_transaction(plan, recipient_address)?;
    sign_p2qrh_transaction(&mut transaction, &plan.inputs, wallet_entropy)?;

    Ok(SignedTransaction {
        transaction_id: transaction.compute_txid().to_string(),
        transaction_hex: encode::serialize_hex(&transaction),
    })
}

pub fn sign_p2qrh_transaction(
    transaction: &mut Transaction,
    selected_utxos: &[VerifiedUtxo],
    wallet_entropy: &[u8; 16],
) -> Result<(), String> {
    verify_inputs_match(transaction, selected_utxos)?;

    let previous_outputs = selected_utxos
        .iter()
        .map(|utxo| TxOut {
            value: utxo.amount,
            script_pubkey: utxo.script_pubkey.clone(),
        })
        .collect::<Vec<_>>();
    let previous_outputs = Prevouts::All(&previous_outputs);
    let secp = Secp256k1::new();

    for (input_index, utxo) in selected_utxos.iter().enumerate() {
        let private_key = match utxo.keychain {
            WalletKeychain::Receiving => keys::derive_receiving_private_key_from_entropy(
                wallet_entropy,
                utxo.derivation_index,
            )?,
            WalletKeychain::Change => {
                keys::derive_change_private_key_from_entropy(wallet_entropy, utxo.derivation_index)?
            }
        };
        let keypair = Keypair::from_secret_key(&secp, &private_key);
        let (public_key, _parity) = keypair.x_only_public_key();
        let public_key_bytes = public_key.serialize();
        let expected_script = p2qrh::script_pubkey_from_public_key(&public_key_bytes);

        if utxo.script_pubkey.as_bytes() != expected_script {
            return Err("selected UTXO does not match its wallet key".to_owned());
        }

        let sighash = SighashCache::new(&*transaction)
            .taproot_key_spend_signature_hash(
                input_index,
                &previous_outputs,
                TapSighashType::Default,
            )
            .map_err(|error| format!("could not create transaction signature hash: {error}"))?;
        let message = Message::from(sighash);
        let signature = secp.sign_schnorr_no_aux_rand(&message, &keypair);

        let mut witness = Witness::new();
        witness.push(signature.as_ref());
        witness.push(public_key_bytes);
        witness.push([SCHNORR_SCHEME_ID]);
        transaction.input[input_index].witness = witness;
    }

    Ok(())
}

fn verify_inputs_match(
    transaction: &Transaction,
    selected_utxos: &[VerifiedUtxo],
) -> Result<(), String> {
    if transaction.input.len() != selected_utxos.len() {
        return Err("transaction inputs do not match the selected UTXOs".to_owned());
    }

    for (transaction_input, selected_utxo) in transaction.input.iter().zip(selected_utxos) {
        if transaction_input.previous_output != selected_utxo.outpoint {
            return Err("transaction inputs do not match the selected UTXOs".to_owned());
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "sign_tests.rs"]
mod tests;
