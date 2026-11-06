use bitcoin::absolute;
use bitcoin::transaction::Version;
use bitcoin::{Amount, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};

use crate::config::WALLET_NETWORK;
use crate::p2qrh;
use crate::transaction_plan::TransactionPlan;

pub fn build_unsigned_transaction(
    plan: &TransactionPlan,
    recipient_address: &str,
) -> Result<Transaction, String> {
    let recipient_script = p2qrh::script_pubkey_from_address(recipient_address, WALLET_NETWORK)?;
    let mut inputs = Vec::with_capacity(plan.inputs.len());

    for selected_utxo in &plan.inputs {
        inputs.push(TxIn {
            previous_output: selected_utxo.outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        });
    }

    let mut outputs = vec![TxOut {
        value: Amount::from_sat(plan.recipient_amount),
        script_pubkey: ScriptBuf::from_bytes(recipient_script.to_vec()),
    }];

    if plan.change > 0 {
        let change_address = plan
            .change_address
            .as_deref()
            .ok_or_else(|| "transaction plan is missing its change address".to_owned())?;
        let change_script = p2qrh::script_pubkey_from_address(change_address, WALLET_NETWORK)?;

        outputs.push(TxOut {
            value: Amount::from_sat(plan.change),
            script_pubkey: ScriptBuf::from_bytes(change_script.to_vec()),
        });
    }

    verify_planned_fee(plan, &outputs)?;

    Ok(Transaction {
        version: Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: inputs,
        output: outputs,
    })
}

fn verify_planned_fee(plan: &TransactionPlan, outputs: &[TxOut]) -> Result<(), String> {
    let mut input_value = 0_u64;

    for input in &plan.inputs {
        input_value = input_value
            .checked_add(input.amount.to_sat())
            .ok_or_else(|| "transaction input value is too large".to_owned())?;
    }

    let mut output_value = 0_u64;

    for output in outputs {
        output_value = output_value
            .checked_add(output.value.to_sat())
            .ok_or_else(|| "transaction output value is too large".to_owned())?;
    }

    let actual_fee = input_value
        .checked_sub(output_value)
        .ok_or_else(|| "transaction outputs exceed its inputs".to_owned())?;

    if actual_fee != plan.fee {
        return Err("transaction outputs do not match the planned fee".to_owned());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::{Amount, OutPoint, ScriptBuf, Txid};

    use super::build_unsigned_transaction;
    use crate::config::WALLET_NETWORK;
    use crate::local_wallet::{VerifiedUtxo, WalletKeychain};
    use crate::p2qrh;
    use crate::transaction_plan::TransactionPlan;

    fn test_address() -> String {
        p2qrh::address_from_public_key(&[2_u8; 32], WALLET_NETWORK)
            .expect("test address should encode for the selected network")
    }

    fn transaction_plan() -> TransactionPlan {
        let outpoint = OutPoint::new(
            Txid::from_str("fd2344a47141b786846d7b9a78b1c76ed62375a27556f78e7e85bc128056ba4e")
                .expect("test transaction ID should be valid"),
            0,
        );
        let input = VerifiedUtxo {
            outpoint,
            amount: Amount::from_sat(1_000_000_000),
            script_pubkey: ScriptBuf::new(),
            keychain: WalletKeychain::Receiving,
            derivation_index: 0,
            confirmations: 1,
            coinbase: false,
        };

        TransactionPlan {
            inputs: vec![input],
            recipient_amount: 100_000_000,
            fee: 1_640,
            change: 899_998_360,
            change_address: Some(test_address()),
        }
    }

    #[test]
    fn unsigned_transaction_matches_the_reviewed_plan() {
        let plan = transaction_plan();
        let transaction = build_unsigned_transaction(&plan, &test_address())
            .expect("reviewed plan should produce a transaction");

        assert_eq!(transaction.input.len(), 1);
        assert_eq!(
            transaction.input[0].previous_output,
            plan.inputs[0].outpoint
        );
        assert!(transaction.input[0].witness.is_empty());
        assert_eq!(transaction.output.len(), 2);
        assert_eq!(transaction.output[0].value.to_sat(), 100_000_000);
        assert_eq!(transaction.output[1].value.to_sat(), 899_998_360);
    }

    #[test]
    fn mismatched_fee_is_rejected() {
        let mut plan = transaction_plan();
        plan.fee = 1_639;

        let error = build_unsigned_transaction(&plan, &test_address())
            .expect_err("changed fee must not produce a transaction");

        assert_eq!(error, "transaction outputs do not match the planned fee");
    }
}
