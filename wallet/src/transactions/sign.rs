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
mod tests {
    use std::str::FromStr;

    use bip39::Mnemonic;
    use bitcoin::secp256k1::{Message, Secp256k1, XOnlyPublicKey, schnorr};
    use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
    use bitcoin::{Amount, OutPoint, ScriptBuf, Transaction, TxOut, Txid};

    use super::sign_p2qrh_transaction;
    use crate::config::WALLET_NETWORK;
    use crate::crypto::{keys, p2qrh};
    use crate::transactions::build::build_unsigned_transaction;
    use crate::transactions::plan::{TransactionPlan, create_transaction_plan};
    use crate::transactions::utxo::{VerifiedUtxo, WalletKeychain};

    #[test]
    fn p2qrh_witness_contains_a_valid_signature_public_key_and_scheme() {
        let (entropy, mut transaction, input) = unsigned_receiving_spend();
        let public_key = keys::derive_receiving_public_key_from_entropy(&entropy, 0)
            .expect("receiving key should derive");
        let previous_outputs = [TxOut {
            value: input.amount,
            script_pubkey: input.script_pubkey.clone(),
        }];
        let sighash = SighashCache::new(&transaction)
            .taproot_key_spend_signature_hash(
                0,
                &Prevouts::All(&previous_outputs),
                TapSighashType::Default,
            )
            .expect("test transaction should have a sighash");

        sign_p2qrh_transaction(&mut transaction, std::slice::from_ref(&input), &entropy)
            .expect("wallet should sign its input");

        let witness_items = transaction.input[0].witness.iter().collect::<Vec<_>>();
        assert_eq!(witness_items.len(), 3);
        assert_eq!(witness_items[0].len(), 64);
        assert_eq!(witness_items[1], public_key);
        assert_eq!(witness_items[2], [0x01]);

        let signature = schnorr::Signature::from_slice(witness_items[0])
            .expect("witness should contain a Schnorr signature");
        let public_key = XOnlyPublicKey::from_slice(witness_items[1])
            .expect("witness should contain an x-only public key");
        let message = Message::from(sighash);

        Secp256k1::verification_only()
            .verify_schnorr(&signature, &message, &public_key)
            .expect("witness signature should verify");
    }

    #[test]
    fn wallet_refuses_to_sign_with_the_wrong_derivation_index() {
        let (entropy, mut transaction, mut input) = unsigned_receiving_spend();
        input.derivation_index = 1;

        let error = sign_p2qrh_transaction(&mut transaction, &[input], &entropy)
            .expect_err("wrong derivation index must not sign");

        assert_eq!(error, "selected UTXO does not match its wallet key");
        assert!(transaction.input[0].witness.is_empty());
    }

    #[test]
    fn wallet_refuses_to_sign_with_another_wallets_entropy() {
        let (_, mut transaction, input) = unsigned_receiving_spend();

        let error = sign_p2qrh_transaction(&mut transaction, &[input], &[1_u8; 16])
            .expect_err("another wallet must not sign this input");

        assert_eq!(error, "selected UTXO does not match its wallet key");
        assert!(transaction.input[0].witness.is_empty());
    }

    #[test]
    fn wallet_refuses_to_sign_with_the_wrong_keychain() {
        let (entropy, mut transaction, mut input) = unsigned_receiving_spend();
        input.keychain = WalletKeychain::Change;

        let error = sign_p2qrh_transaction(&mut transaction, &[input], &entropy)
            .expect_err("a change key must not sign a receiving output");

        assert_eq!(error, "selected UTXO does not match its wallet key");
        assert!(transaction.input[0].witness.is_empty());
    }

    #[test]
    fn wallet_refuses_to_sign_a_mismatched_commitment() {
        let (entropy, mut transaction, mut input) = unsigned_receiving_spend();
        let mut script = input.script_pubkey.to_bytes();
        script[2] ^= 1;
        input.script_pubkey = ScriptBuf::from_bytes(script);

        let error = sign_p2qrh_transaction(&mut transaction, &[input], &entropy)
            .expect_err("the key must match the actual output commitment");

        assert_eq!(error, "selected UTXO does not match its wallet key");
        assert!(transaction.input[0].witness.is_empty());
    }

    #[test]
    fn wallet_refuses_to_sign_without_the_selected_utxo() {
        let (entropy, mut transaction, _) = unsigned_receiving_spend();

        let error = sign_p2qrh_transaction(&mut transaction, &[], &entropy)
            .expect_err("every input requires its selected UTXO");

        assert_eq!(error, "transaction inputs do not match the selected UTXOs");
        assert!(transaction.input[0].witness.is_empty());
    }

    #[test]
    fn wallet_refuses_to_sign_a_different_outpoint() {
        let (entropy, mut transaction, mut input) = unsigned_receiving_spend();
        input.outpoint.vout += 1;

        let error = sign_p2qrh_transaction(&mut transaction, &[input], &entropy)
            .expect_err("selected UTXO must identify the transaction input");

        assert_eq!(error, "transaction inputs do not match the selected UTXOs");
        assert!(transaction.input[0].witness.is_empty());
    }

    #[test]
    fn signing_preserves_the_reviewed_transaction_details() {
        let (entropy, mut transaction, input) = unsigned_receiving_spend();
        let reviewed_transaction = transaction.clone();

        sign_p2qrh_transaction(&mut transaction, &[input], &entropy)
            .expect("wallet should sign its input");

        assert!(!transaction.input[0].witness.is_empty());
        transaction.input[0].witness.clear();
        assert_eq!(transaction, reviewed_transaction);
    }

    #[test]
    fn signature_commits_to_the_recipient_amount_fee_and_previous_output() {
        let (entropy, mut transaction, input) = unsigned_receiving_spend();
        sign_p2qrh_transaction(&mut transaction, std::slice::from_ref(&input), &entropy)
            .expect("wallet should sign its input");
        assert!(signature_verifies(
            &transaction,
            std::slice::from_ref(&input),
            0
        ));

        let mut changed_recipient = transaction.clone();
        changed_recipient.output[0].script_pubkey = transaction.output[1].script_pubkey.clone();
        assert!(!signature_verifies(
            &changed_recipient,
            std::slice::from_ref(&input),
            0
        ));

        let mut changed_amount = transaction.clone();
        changed_amount.output[0].value = Amount::from_sat(100_000_001);
        assert!(!signature_verifies(
            &changed_amount,
            std::slice::from_ref(&input),
            0
        ));

        let mut changed_fee = transaction.clone();
        changed_fee.output[1].value = Amount::from_sat(899_998_359);
        assert!(!signature_verifies(
            &changed_fee,
            std::slice::from_ref(&input),
            0
        ));

        let mut changed_input_amount = input.clone();
        changed_input_amount.amount = Amount::from_sat(1_000_000_001);
        assert!(!signature_verifies(
            &transaction,
            &[changed_input_amount],
            0
        ));

        let mut changed_input_script = input;
        let mut script = changed_input_script.script_pubkey.to_bytes();
        script[2] ^= 1;
        changed_input_script.script_pubkey = ScriptBuf::from_bytes(script);
        assert!(!signature_verifies(
            &transaction,
            &[changed_input_script],
            0
        ));
    }

    #[test]
    fn multiple_utxos_and_repeated_change_spends_are_signed() {
        let mnemonic = Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .expect("test phrase should be valid");
        let entropy: [u8; 16] = mnemonic
            .to_entropy()
            .try_into()
            .expect("test phrase should contain 16 bytes of entropy");
        let first_input = receiving_utxo(
            &entropy,
            0,
            60_000_000,
            "fd2344a47141b786846d7b9a78b1c76ed62375a27556f78e7e85bc128056ba4e",
        );
        let second_input = receiving_utxo(
            &entropy,
            1,
            50_000_000,
            "97e195eafc5824409588218a47ccf75e4377bc43555ffb30f80250c1f24124ff",
        );
        let recipient_key = keys::derive_receiving_public_key_from_entropy(&entropy, 8)
            .expect("recipient key should derive");
        let recipient = p2qrh::address_from_public_key(&recipient_key, WALLET_NETWORK)
            .expect("recipient address should encode");
        let first_change_key = keys::derive_change_public_key_from_entropy(&entropy, 0)
            .expect("first change key should derive");
        let first_change_address =
            p2qrh::address_from_public_key(&first_change_key, WALLET_NETWORK)
                .expect("first change address should encode");

        let first_plan = create_transaction_plan(
            vec![first_input, second_input],
            100_000_000,
            10_000,
            first_change_address,
        )
        .expect("two inputs should fund the first payment");
        assert_eq!(first_plan.inputs.len(), 2);
        let mut first_transaction = build_unsigned_transaction(&first_plan, &recipient)
            .expect("first transaction should build");
        sign_p2qrh_transaction(&mut first_transaction, &first_plan.inputs, &entropy)
            .expect("both receiving inputs should sign");
        for input_index in 0..first_plan.inputs.len() {
            let expected_key = keys::derive_receiving_public_key_from_entropy(
                &entropy,
                first_plan.inputs[input_index].derivation_index,
            )
            .expect("receiving key should derive");
            let witness = first_transaction.input[input_index].witness.to_vec();
            assert_eq!(witness[1], expected_key);
            assert!(signature_verifies(
                &first_transaction,
                &first_plan.inputs,
                input_index
            ));
        }

        let first_change_output = VerifiedUtxo {
            outpoint: OutPoint::new(first_transaction.compute_txid(), 1),
            amount: Amount::from_sat(first_plan.change),
            script_pubkey: ScriptBuf::from_bytes(
                p2qrh::script_pubkey_from_public_key(&first_change_key).to_vec(),
            ),
            keychain: WalletKeychain::Change,
            derivation_index: 0,
            confirmations: 1,
            coinbase: false,
        };
        let second_change_key = keys::derive_change_public_key_from_entropy(&entropy, 1)
            .expect("second change key should derive");
        let second_change_address =
            p2qrh::address_from_public_key(&second_change_key, WALLET_NETWORK)
                .expect("second change address should encode");
        let second_plan = create_transaction_plan(
            vec![first_change_output],
            5_000_000,
            10_000,
            second_change_address,
        )
        .expect("the first transaction's change should fund a second payment");
        let mut second_transaction = build_unsigned_transaction(&second_plan, &recipient)
            .expect("second transaction should build");
        sign_p2qrh_transaction(&mut second_transaction, &second_plan.inputs, &entropy)
            .expect("the change input should sign with the change branch");

        assert_eq!(second_plan.inputs[0].keychain, WalletKeychain::Change);
        assert_eq!(second_plan.inputs[0].derivation_index, 0);
        let change_witness = second_transaction.input[0].witness.to_vec();
        assert_eq!(change_witness[1], first_change_key);
        assert!(signature_verifies(
            &second_transaction,
            &second_plan.inputs,
            0
        ));
    }

    fn signature_verifies(
        transaction: &Transaction,
        inputs: &[VerifiedUtxo],
        input_index: usize,
    ) -> bool {
        let mut previous_outputs = Vec::new();
        for input in inputs {
            previous_outputs.push(TxOut {
                value: input.amount,
                script_pubkey: input.script_pubkey.clone(),
            });
        }
        let sighash = SighashCache::new(transaction)
            .taproot_key_spend_signature_hash(
                input_index,
                &Prevouts::All(&previous_outputs),
                TapSighashType::Default,
            )
            .expect("test transaction should have a sighash");
        let witness = transaction.input[input_index].witness.to_vec();
        let signature = schnorr::Signature::from_slice(&witness[0])
            .expect("witness should contain a Schnorr signature");
        let public_key = XOnlyPublicKey::from_slice(&witness[1])
            .expect("witness should contain an x-only public key");
        Secp256k1::verification_only()
            .verify_schnorr(&signature, &Message::from(sighash), &public_key)
            .is_ok()
    }

    fn receiving_utxo(
        entropy: &[u8; 16],
        derivation_index: u32,
        amount: u64,
        transaction_id: &str,
    ) -> VerifiedUtxo {
        let public_key = keys::derive_receiving_public_key_from_entropy(entropy, derivation_index)
            .expect("receiving key should derive");

        VerifiedUtxo {
            outpoint: OutPoint::new(
                Txid::from_str(transaction_id).expect("test transaction ID should be valid"),
                0,
            ),
            amount: Amount::from_sat(amount),
            script_pubkey: ScriptBuf::from_bytes(
                p2qrh::script_pubkey_from_public_key(&public_key).to_vec(),
            ),
            keychain: WalletKeychain::Receiving,
            derivation_index,
            confirmations: 1,
            coinbase: false,
        }
    }

    fn unsigned_receiving_spend() -> ([u8; 16], Transaction, VerifiedUtxo) {
        let entropy = [0_u8; 16];
        let input = receiving_utxo(
            &entropy,
            0,
            1_000_000_000,
            "fd2344a47141b786846d7b9a78b1c76ed62375a27556f78e7e85bc128056ba4e",
        );
        let recipient_key = keys::derive_receiving_public_key_from_entropy(&entropy, 1)
            .expect("recipient key should derive");
        let recipient = p2qrh::address_from_public_key(&recipient_key, WALLET_NETWORK)
            .expect("recipient address should encode");
        let change_key = keys::derive_change_public_key_from_entropy(&entropy, 0)
            .expect("change key should derive");
        let change_address = p2qrh::address_from_public_key(&change_key, WALLET_NETWORK)
            .expect("change address should encode");
        let plan = TransactionPlan {
            inputs: vec![input.clone()],
            recipient_amount: 100_000_000,
            fee: 1_640,
            change: 899_998_360,
            change_address: Some(change_address),
        };
        let transaction = build_unsigned_transaction(&plan, &recipient)
            .expect("plan should produce an unsigned transaction");
        (entropy, transaction, input)
    }
}
