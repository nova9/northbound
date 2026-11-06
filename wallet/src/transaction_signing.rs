use bitcoin::secp256k1::{Keypair, Message, Secp256k1};
use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
use bitcoin::{Transaction, TxOut, Witness};

use crate::keys;
use crate::local_wallet::{VerifiedUtxo, WalletKeychain};
use crate::p2qrh;

const SCHNORR_SCHEME_ID: u8 = 0x01;

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
    use bitcoin::{Amount, OutPoint, ScriptBuf, TxOut, Txid};

    use super::sign_p2qrh_transaction;
    use crate::config::WALLET_NETWORK;
    use crate::keys;
    use crate::local_wallet::{VerifiedUtxo, WalletKeychain};
    use crate::p2qrh;
    use crate::transaction_plan::{TransactionPlan, create_transaction_plan};
    use crate::unsigned_transaction::build_unsigned_transaction;

    #[test]
    fn p2qrh_witness_contains_a_valid_signature_public_key_and_scheme() {
        let mnemonic = Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .expect("test phrase should be valid");
        let entropy: [u8; 16] = mnemonic
            .to_entropy()
            .try_into()
            .expect("test phrase should contain 16 bytes of entropy");
        let public_key = keys::derive_receiving_public_key_from_entropy(&entropy, 0)
            .expect("receiving key should derive");
        let input_script = p2qrh::script_pubkey_from_public_key(&public_key);
        let input = VerifiedUtxo {
            outpoint: OutPoint::new(
                Txid::from_str("fd2344a47141b786846d7b9a78b1c76ed62375a27556f78e7e85bc128056ba4e")
                    .expect("test transaction ID should be valid"),
                0,
            ),
            amount: Amount::from_sat(1_000_000_000),
            script_pubkey: ScriptBuf::from_bytes(input_script.to_vec()),
            keychain: WalletKeychain::Receiving,
            derivation_index: 0,
            confirmations: 1,
            coinbase: false,
        };
        let change_address = p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)
            .expect("change fixture should be a valid address");
        let plan = TransactionPlan {
            inputs: vec![input.clone()],
            recipient_amount: 100_000_000,
            fee: 1_640,
            change: 899_998_360,
            change_address: Some(change_address),
        };
        let recipient = p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)
            .expect("recipient fixture should be a valid address");
        let mut transaction = build_unsigned_transaction(&plan, &recipient)
            .expect("plan should produce an unsigned transaction");
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

        sign_p2qrh_transaction(&mut transaction, &plan.inputs, &entropy)
            .expect("wallet should sign its input");

        let witness_items = transaction.input[0].witness.iter().collect::<Vec<_>>();
        assert_eq!(witness_items.len(), 3);
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
        let mnemonic = Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .expect("test phrase should be valid");
        let entropy: [u8; 16] = mnemonic
            .to_entropy()
            .try_into()
            .expect("test phrase should contain 16 bytes of entropy");
        let public_key = keys::derive_receiving_public_key_from_entropy(&entropy, 0)
            .expect("receiving key should derive");
        let input_script = p2qrh::script_pubkey_from_public_key(&public_key);
        let input = VerifiedUtxo {
            outpoint: OutPoint::new(
                Txid::from_str("fd2344a47141b786846d7b9a78b1c76ed62375a27556f78e7e85bc128056ba4e")
                    .expect("test transaction ID should be valid"),
                0,
            ),
            amount: Amount::from_sat(1_000_000_000),
            script_pubkey: ScriptBuf::from_bytes(input_script.to_vec()),
            keychain: WalletKeychain::Receiving,
            derivation_index: 1,
            confirmations: 1,
            coinbase: false,
        };
        let change_address = p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)
            .expect("change fixture should be a valid address");
        let plan = TransactionPlan {
            inputs: vec![input],
            recipient_amount: 100_000_000,
            fee: 1_640,
            change: 899_998_360,
            change_address: Some(change_address),
        };
        let recipient = p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)
            .expect("recipient fixture should be a valid address");
        let mut transaction = build_unsigned_transaction(&plan, &recipient)
            .expect("plan should produce an unsigned transaction");

        let error = sign_p2qrh_transaction(&mut transaction, &plan.inputs, &entropy)
            .expect_err("wrong derivation index must not sign");

        assert_eq!(error, "selected UTXO does not match its wallet key");
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
            10,
            first_change_address,
        )
        .expect("two inputs should fund the first payment");
        assert_eq!(first_plan.inputs.len(), 2);
        let mut first_transaction = build_unsigned_transaction(&first_plan, &recipient)
            .expect("first transaction should build");
        sign_p2qrh_transaction(&mut first_transaction, &first_plan.inputs, &entropy)
            .expect("both receiving inputs should sign");
        assert!(
            first_transaction
                .input
                .iter()
                .all(|input| !input.witness.is_empty())
        );

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
            10,
            second_change_address,
        )
        .expect("the first transaction's change should fund a second payment");
        let mut second_transaction = build_unsigned_transaction(&second_plan, &recipient)
            .expect("second transaction should build");
        sign_p2qrh_transaction(&mut second_transaction, &second_plan.inputs, &entropy)
            .expect("the change input should sign with the change branch");

        assert_eq!(second_plan.inputs[0].keychain, WalletKeychain::Change);
        assert_eq!(second_plan.inputs[0].derivation_index, 0);
        assert!(!second_transaction.input[0].witness.is_empty());
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
}
