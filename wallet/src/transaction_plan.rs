use crate::local_wallet::VerifiedUtxo;

const P2QRH_DUST_LIMIT: u64 = 330;
const P2QRH_ONE_INPUT_ONE_OUTPUT_VBYTES: u64 = 121;
const ADDITIONAL_P2QRH_INPUT_VBYTES: u64 = 67;
const ADDITIONAL_P2QRH_OUTPUT_VBYTES: u64 = 43;

#[derive(Clone, Debug)]
pub struct TransactionPlan {
    pub inputs: Vec<VerifiedUtxo>,
    pub recipient_amount: u64,
    pub fee: u64,
    pub change: u64,
    pub change_address: Option<String>,
}

pub fn create_transaction_plan(
    mut available_utxos: Vec<VerifiedUtxo>,
    recipient_amount: u64,
    fee_rate: u64,
    change_address: String,
) -> Result<TransactionPlan, String> {
    if recipient_amount < P2QRH_DUST_LIMIT {
        return Err(format!(
            "payment amount must be at least {P2QRH_DUST_LIMIT} atomic units"
        ));
    }

    if fee_rate == 0 {
        return Err("fee rate must be greater than zero".to_owned());
    }

    available_utxos.retain(VerifiedUtxo::is_spendable);
    available_utxos.sort_by_key(|utxo| std::cmp::Reverse(utxo.amount));

    let mut selected_inputs = Vec::new();
    let mut selected_value = 0_u64;

    for utxo in available_utxos {
        selected_value = selected_value
            .checked_add(utxo.amount.to_sat())
            .ok_or_else(|| "selected input value is too large".to_owned())?;
        selected_inputs.push(utxo);

        let input_count = selected_inputs.len();
        let fee_with_change = estimated_fee(input_count, true, fee_rate)?;
        let required_with_change = recipient_amount
            .checked_add(fee_with_change)
            .ok_or_else(|| "payment amount and fee are too large".to_owned())?;

        if selected_value >= required_with_change {
            let change = selected_value - required_with_change;

            if change >= P2QRH_DUST_LIMIT {
                return Ok(TransactionPlan {
                    inputs: selected_inputs,
                    recipient_amount,
                    fee: fee_with_change,
                    change,
                    change_address: Some(change_address),
                });
            }

            return Ok(TransactionPlan {
                inputs: selected_inputs,
                recipient_amount,
                fee: selected_value - recipient_amount,
                change: 0,
                change_address: None,
            });
        }

        let fee_without_change = estimated_fee(input_count, false, fee_rate)?;
        let required_without_change = recipient_amount
            .checked_add(fee_without_change)
            .ok_or_else(|| "payment amount and fee are too large".to_owned())?;

        if selected_value >= required_without_change {
            let remainder = selected_value - required_without_change;

            if remainder < P2QRH_DUST_LIMIT {
                return Ok(TransactionPlan {
                    inputs: selected_inputs,
                    recipient_amount,
                    fee: selected_value - recipient_amount,
                    change: 0,
                    change_address: None,
                });
            }
        }
    }

    Err("insufficient confirmed funds for the payment and fee".to_owned())
}

fn estimated_fee(input_count: usize, include_change: bool, fee_rate: u64) -> Result<u64, String> {
    let additional_inputs = input_count
        .checked_sub(1)
        .ok_or_else(|| "transaction must contain at least one input".to_owned())?;
    let additional_input_vbytes = (additional_inputs as u64)
        .checked_mul(ADDITIONAL_P2QRH_INPUT_VBYTES)
        .ok_or_else(|| "transaction size is too large".to_owned())?;
    let change_vbytes = if include_change {
        ADDITIONAL_P2QRH_OUTPUT_VBYTES
    } else {
        0
    };
    let estimated_vbytes = P2QRH_ONE_INPUT_ONE_OUTPUT_VBYTES
        .checked_add(additional_input_vbytes)
        .and_then(|size| size.checked_add(change_vbytes))
        .ok_or_else(|| "transaction size is too large".to_owned())?;

    estimated_vbytes
        .checked_mul(fee_rate)
        .ok_or_else(|| "transaction fee is too large".to_owned())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::{Amount, OutPoint, ScriptBuf, Txid};

    use super::create_transaction_plan;
    use crate::local_wallet::{VerifiedUtxo, WalletKeychain};

    fn confirmed_utxo(amount: u64, output_index: u32) -> VerifiedUtxo {
        VerifiedUtxo {
            outpoint: OutPoint::new(
                Txid::from_str("fd2344a47141b786846d7b9a78b1c76ed62375a27556f78e7e85bc128056ba4e")
                    .expect("test transaction ID should be valid"),
                output_index,
            ),
            amount: Amount::from_sat(amount),
            script_pubkey: ScriptBuf::new(),
            keychain: WalletKeychain::Receiving,
            derivation_index: 0,
            confirmations: 1,
            coinbase: false,
        }
    }

    #[test]
    fn ten_north_utxo_plans_one_north_payment_with_change() {
        let plan = create_transaction_plan(
            vec![confirmed_utxo(1_000_000_000, 0)],
            100_000_000,
            10,
            "bcrt1zchange".to_owned(),
        )
        .expect("payment should be affordable");

        assert_eq!(plan.inputs.len(), 1);
        assert_eq!(plan.recipient_amount, 100_000_000);
        assert_eq!(plan.fee, 1_640);
        assert_eq!(plan.change, 899_998_360);
        assert_eq!(plan.change_address.as_deref(), Some("bcrt1zchange"));
    }

    #[test]
    fn dust_change_is_added_to_the_fee() {
        let plan = create_transaction_plan(
            vec![confirmed_utxo(10_000, 0)],
            8_690,
            10,
            "bcrt1zchange".to_owned(),
        )
        .expect("payment should use its dust remainder as fee");

        assert_eq!(plan.fee, 1_310);
        assert_eq!(plan.change, 0);
        assert_eq!(plan.change_address, None);
    }

    #[test]
    fn insufficient_funds_are_rejected() {
        let error = create_transaction_plan(
            vec![confirmed_utxo(100_000_000, 0)],
            100_000_000,
            10,
            "bcrt1zchange".to_owned(),
        )
        .expect_err("fee should make the payment unaffordable");

        assert_eq!(
            error,
            "insufficient confirmed funds for the payment and fee"
        );
    }
}
