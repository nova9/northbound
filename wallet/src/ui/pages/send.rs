use bitcoin::consensus::encode;
use dioxus::prelude::*;

use crate::config::{CURRENCY_TICKER, WALLET_NETWORK};
use crate::local_wallet::{UnlockedWallet, VerifiedUtxo, calculate_spendable_balance};
use crate::p2qrh;
use crate::transaction_plan::{TransactionPlan, create_transaction_plan};
use crate::ui::state::AppContext;
use crate::unsigned_transaction::build_unsigned_transaction;

const ATOMIC_UNITS_PER_NORTH: u64 = 100_000_000;

#[derive(Clone)]
enum SendState {
    Ready,
    Failed(String),
    Sending,
    Sent(String),
    Uncertain(String),
}

struct SignedTransaction {
    transaction_hex: String,
    transaction_id: String,
}

#[component]
pub(crate) fn Send() -> Element {
    let mut context = use_context::<AppContext>();
    let mut recipient = use_signal(String::new);
    let mut amount = use_signal(String::new);
    let mut validation_error = use_signal(|| None::<String>);
    let mut transaction_plan = use_signal(|| None::<TransactionPlan>);
    let mut send_state = use_signal(|| SendState::Ready);

    let addresses = {
        let wallet = context.unlocked_wallet.read();

        match wallet.as_ref() {
            Some(wallet) => wallet.tracked_addresses(),
            None => Err("Unlock the wallet again to load spendable funds.".to_owned()),
        }
    };

    let utxos = use_resource(move || {
        let addresses = addresses.clone();

        async move {
            let addresses = addresses?;
            crate::api::fetch_utxos(addresses).await
        }
    });
    let fee_estimate = use_resource(crate::api::fetch_fee_estimate);

    let verified_utxos_state = match &*utxos.read() {
        None => None,
        Some(Err(message)) => Some(Err(message.clone())),
        Some(Ok(utxos)) => {
            let wallet = context.unlocked_wallet.read();
            let verified_utxos = wallet
                .as_ref()
                .ok_or_else(|| "Unlock the wallet again to verify spendable funds.".to_owned())
                .and_then(|wallet| wallet.verify_utxos(utxos.clone()));

            Some(verified_utxos)
        }
    };
    let available_balance = verified_utxos_state
        .as_ref()
        .and_then(|utxos| utxos.as_ref().ok())
        .and_then(|utxos| calculate_spendable_balance(utxos).ok());
    let fee_estimate_state = fee_estimate.read().clone();
    let atomic_units_per_kvbyte = fee_estimate_state
        .as_ref()
        .and_then(|estimate| estimate.as_ref().ok())
        .map(|estimate| estimate.atomic_units_per_kvbyte);

    let send_locked = matches!(
        send_state(),
        SendState::Sending | SendState::Sent(_) | SendState::Uncertain(_)
    );

    let entered_amount = parse_north(&amount()).ok();
    let reviewed_plan = transaction_plan();
    let reviewed_fee = reviewed_plan.as_ref().map(|plan| plan.fee);
    let reviewed_total = reviewed_plan
        .as_ref()
        .and_then(|plan| plan.recipient_amount.checked_add(plan.fee));
    let reviewed_remaining = reviewed_total.and_then(|total| available_balance?.checked_sub(total));
    let selected_input_label = reviewed_plan.as_ref().map(|plan| {
        if plan.inputs.len() == 1 {
            "1 selected input".to_owned()
        } else {
            format!("{} selected inputs", plan.inputs.len())
        }
    });
    let fee_warning = reviewed_plan.as_ref().and_then(|plan| {
        fee_is_large_share_of_payment(plan.fee, plan.recipient_amount).then(|| {
            "The fee is at least 10% of the payment amount. Check it carefully before signing."
                .to_owned()
        })
    });

    rsx! {
        section { class: "screen send-screen",
            div { class: "send-heading",
                div {
                    h1 { "Send {CURRENCY_TICKER}" }
                    p { class: "lede", "Enter the payment details, then review the full transaction before signing." }
                }
            }

            div { class: "send-layout",
                form {
                    class: "send-form",
                    onsubmit: move |event| {
                        event.prevent_default();
                        if matches!(send_state(), SendState::Sending | SendState::Sent(_) | SendState::Uncertain(_)) {
                            return;
                        }

                        let result = prepare_transaction_plan(
                            &recipient(),
                            &amount(),
                            &verified_utxos_state,
                            atomic_units_per_kvbyte,
                            &context,
                        );

                        match result {
                            Ok(plan) => {
                                transaction_plan.set(Some(plan));
                                send_state.set(SendState::Ready);
                                validation_error.set(None);
                            }
                            Err(message) => {
                                transaction_plan.set(None);
                                send_state.set(SendState::Ready);
                                validation_error.set(Some(message));
                            }
                        }
                    },
                    label { class: "field",
                        span { "Recipient address" }
                        input {
                            r#type: "text",
                            value: "{recipient}",
                            placeholder: "Paste a {CURRENCY_TICKER} address",
                            autocomplete: "off",
                            disabled: send_locked,
                            spellcheck: "false",
                            oninput: move |event| {
                                recipient.set(event.value());
                                transaction_plan.set(None);
                                send_state.set(SendState::Ready);
                            },
                        }
                        small { "Check the complete address before continuing." }
                    }

                    label { class: "field",
                        span { "Amount" }
                        div { class: "amount-input",
                            input {
                                r#type: "text",
                                inputmode: "decimal",
                                value: "{amount}",
                                placeholder: "0.00000000",
                                autocomplete: "off",
                                disabled: send_locked,
                                oninput: move |event| {
                                    amount.set(event.value());
                                    transaction_plan.set(None);
                                    send_state.set(SendState::Ready);
                                },
                            }
                            span { "{CURRENCY_TICKER}" }
                        }
                        small {
                            match &verified_utxos_state {
                                None => "Loading spendable balance…".to_owned(),
                                Some(Err(_)) => "Spendable balance unavailable".to_owned(),
                                Some(Ok(_)) => match available_balance {
                                    Some(balance) => format!("Available: {} {CURRENCY_TICKER}", format_north(balance)),
                                    None => "Spendable balance unavailable".to_owned(),
                                },
                            }
                        }
                    }

                    if let Some(Err(message)) = &verified_utxos_state {
                        p { class: "error-message send-error", role: "alert", "{message}" }
                    }

                    div { class: "fee-choice",
                        div {
                            strong { "Standard fee" }
                            p {
                                match &fee_estimate_state {
                                    None => "Loading node fee estimate…".to_owned(),
                                    Some(Err(_)) => "Fee estimate unavailable".to_owned(),
                                    Some(Ok(estimate)) => format!(
                                        "Node estimate · {} atomic units/kvB · {} block target",
                                        estimate.atomic_units_per_kvbyte,
                                        estimate.target_blocks,
                                    ),
                                }
                            }
                        }
                        span { "{format_optional_north(reviewed_fee)}" }
                    }

                    if let Some(Err(message)) = &fee_estimate_state {
                        p { class: "error-message send-error", role: "alert", "{message}" }
                    }

                    if let Some(message) = validation_error() {
                        p { class: "error-message send-error", role: "alert", "{message}" }
                    }

                    button {
                        class: "button primary",
                        r#type: "submit",
                        disabled: send_locked || available_balance.is_none() || atomic_units_per_kvbyte.is_none(),
                        "Review transaction"
                    }
                }

                aside { class: "send-summary", aria_label: "Transaction summary",
                    h2 { "Transaction summary" }
                    dl {
                        div {
                            dt { "Amount" }
                            dd { "{format_optional_north(reviewed_plan.as_ref().map(|plan| plan.recipient_amount).or(entered_amount))}" }
                        }
                        div {
                            dt { "Estimated fee" }
                            dd { "{format_optional_north(reviewed_fee)}" }
                        }
                        div { class: "send-total",
                            dt { "Total" }
                            dd { "{format_optional_north(reviewed_total)}" }
                        }
                        div {
                            dt { "Remaining balance" }
                            dd { "{format_optional_north(reviewed_remaining)}" }
                        }
                    }

                    if let Some(plan) = reviewed_plan {
                        div { class: "transaction-review", role: "status",
                            strong { "Transaction plan ready" }
                            p { "{selected_input_label.as_deref().unwrap_or_default()}" }
                            if plan.change > 0 {
                                p { "Change: {format_north(plan.change)} {CURRENCY_TICKER}" }
                            } else {
                                p { "No change output is needed." }
                            }
                            if let Some(message) = fee_warning.as_deref() {
                                p { class: "error-message", role: "alert", "{message}" }
                            }
                            if matches!(send_state(), SendState::Ready) {
                                p { "Check the recipient, amount, and fee. Sign and send will authorize and submit this payment." }
                            }

                            match send_state() {
                                SendState::Ready => rsx! {
                                    button {
                                        class: "button primary",
                                        r#type: "button",
                                        onclick: move |_| {
                                            if !matches!(send_state(), SendState::Ready) {
                                                return;
                                            }
                                            send_state.set(SendState::Sending);
                                            let result = {
                                                let mut wallet = context.unlocked_wallet.write();
                                                match wallet.as_mut() {
                                                    Some(wallet) => build_sign_and_persist_transaction(wallet, &plan, &recipient()),
                                                    None => Err("Unlock the wallet again to send the transaction.".to_owned()),
                                                }
                                            };

                                            let signed_transaction = match result {
                                                Ok(transaction) => transaction,
                                                Err(message) => {
                                                    send_state.set(SendState::Failed(message));
                                                    return;
                                                }
                                            };
                                            spawn(async move {
                                                let expected_transaction_id = signed_transaction.transaction_id;
                                                match crate::api::broadcast_signed_transaction(signed_transaction.transaction_hex).await {
                                                    Ok(transaction_id) => send_state.set(SendState::Sent(transaction_id)),
                                                    Err(broadcast_error) => {
                                                        match crate::api::fetch_transaction_status(expected_transaction_id.clone()).await {
                                                            Ok(true) => send_state.set(SendState::Sent(expected_transaction_id)),
                                                            Ok(false) => send_state.set(SendState::Uncertain(broadcast_error)),
                                                            Err(status_error) => send_state.set(SendState::Uncertain(format!(
                                                                "{broadcast_error} Status check also failed: {status_error}"
                                                            ))),
                                                        }
                                                    }
                                                }
                                            });
                                        },
                                        "Sign and send"
                                    }
                                },
                                SendState::Failed(message) => rsx! {
                                    p { class: "error-message", role: "alert",
                                        "Could not prepare the transaction: {message} Review the transaction again before sending."
                                    }
                                },
                                SendState::Sending => rsx! {
                                    div { role: "status",
                                        button { class: "button primary", r#type: "button", disabled: true,
                                            "Sending…"
                                        }
                                    }
                                },
                                SendState::Sent(transaction_id) => rsx! {
                                    div { role: "status",
                                        p { class: "success-message", "Transaction sent." }
                                        p { class: "transaction-id", "Transaction ID: {transaction_id}" }
                                    }
                                },
                                SendState::Uncertain(message) => rsx! {
                                    p { class: "error-message", role: "alert",
                                        "The send result is uncertain: {message} Check History before trying anything else."
                                    }
                                },
                            }
                        }
                    } else {
                        p { class: "summary-guidance", "Enter valid payment details to prepare the review." }
                    }
                }
            }
        }
    }
}

fn build_sign_and_persist_transaction(
    wallet: &mut UnlockedWallet,
    plan: &TransactionPlan,
    recipient: &str,
) -> Result<SignedTransaction, String> {
    let mut transaction = build_unsigned_transaction(plan, recipient)?;
    if plan.change > 0 {
        let next_change_address = wallet.next_change_address()?;
        if plan.change_address.as_deref() != Some(next_change_address.as_str()) {
            return Err(
                "The wallet change address changed. Review the transaction again.".to_owned(),
            );
        }
    }

    wallet.sign_p2qrh_transaction(&mut transaction, &plan.inputs)?;
    // Persist change before returning any signed bytes that could be broadcast.
    if plan.change > 0 {
        wallet.reserve_and_persist_next_change_address()?;
    }

    Ok(SignedTransaction {
        transaction_id: transaction.compute_txid().to_string(),
        transaction_hex: encode::serialize_hex(&transaction),
    })
}

fn prepare_transaction_plan(
    recipient: &str,
    amount: &str,
    verified_utxos_state: &Option<Result<Vec<VerifiedUtxo>, String>>,
    atomic_units_per_kvbyte: Option<u64>,
    context: &AppContext,
) -> Result<TransactionPlan, String> {
    p2qrh::script_pubkey_from_address(recipient, WALLET_NETWORK).map_err(|message| {
        format!(
            "Enter a valid {} recipient address: {message}.",
            WALLET_NETWORK.name()
        )
    })?;

    let amount = parse_north(amount)?;

    if amount == 0 {
        return Err("Enter an amount greater than zero.".to_owned());
    }

    let atomic_units_per_kvbyte = atomic_units_per_kvbyte
        .ok_or_else(|| "Wait for the network fee estimate to load.".to_owned())?;

    let verified_utxos = verified_utxos_state
        .as_ref()
        .ok_or_else(|| "Wait for the spendable balance to finish loading.".to_owned())?
        .as_ref()
        .map_err(Clone::clone)?
        .clone();

    let wallet = context.unlocked_wallet.read();
    let wallet = wallet
        .as_ref()
        .ok_or_else(|| "Unlock the wallet again to prepare the transaction.".to_owned())?;
    let change_address = wallet.next_change_address()?;

    create_transaction_plan(
        verified_utxos,
        amount,
        atomic_units_per_kvbyte,
        change_address,
    )
}

fn parse_north(value: &str) -> Result<u64, String> {
    let value = value.trim();
    let (whole_part, fractional_part) = value.split_once('.').unwrap_or((value, ""));

    if whole_part.is_empty() || fractional_part.len() > 8 {
        return Err("Enter an amount with no more than eight decimal places.".to_owned());
    }

    let whole_units = whole_part
        .parse::<u64>()
        .map_err(|_| format!("Enter a valid {CURRENCY_TICKER} amount."))?;
    let fractional_units = if fractional_part.is_empty() {
        0
    } else {
        let fraction = fractional_part
            .parse::<u64>()
            .map_err(|_| format!("Enter a valid {CURRENCY_TICKER} amount."))?;
        fraction * 10_u64.pow((8 - fractional_part.len()) as u32)
    };

    whole_units
        .checked_mul(ATOMIC_UNITS_PER_NORTH)
        .and_then(|units| units.checked_add(fractional_units))
        .ok_or_else(|| "The payment amount is too large.".to_owned())
}

fn format_north(amount: u64) -> String {
    let whole_units = amount / ATOMIC_UNITS_PER_NORTH;
    let fractional_units = amount % ATOMIC_UNITS_PER_NORTH;

    format!("{whole_units}.{fractional_units:08}")
}

fn format_optional_north(amount: Option<u64>) -> String {
    match amount {
        Some(amount) => format!("{} {CURRENCY_TICKER}", format_north(amount)),
        None => "—".to_owned(),
    }
}

fn fee_is_large_share_of_payment(fee: u64, payment_amount: u64) -> bool {
    fee >= payment_amount.div_ceil(10)
}

#[cfg(test)]
mod tests {
    use super::{fee_is_large_share_of_payment, parse_north};

    fn wallet_and_plan(
        path: &std::path::Path,
    ) -> (super::UnlockedWallet, super::TransactionPlan, String) {
        let pending = crate::keys::generate_new_wallet().unwrap();
        let mut wallet = crate::local_wallet::save_new_wallet_and_unlock(
            path,
            &pending,
            pending.recovery_phrase(),
            "test password",
            "test password",
        )
        .unwrap();
        let recipient = wallet
            .allocate_and_persist_next_receiving_address()
            .unwrap();
        let script =
            crate::p2qrh::script_pubkey_from_address(&recipient, crate::config::WALLET_NETWORK)
                .unwrap();
        let utxos = wallet
            .verify_utxos(vec![shared::WalletUtxo {
                transaction_id: "ab".repeat(32),
                output_index: 0,
                amount: 100_000,
                script_pubkey: bitcoin::hex::DisplayHex::as_hex(script.as_slice()).to_string(),
                block_height: 1,
                confirmations: 1,
                coinbase: false,
            }])
            .unwrap();
        let change_address = wallet.next_change_address().unwrap();
        let plan = super::create_transaction_plan(utxos, 50_000, 1_001, change_address).unwrap();
        (wallet, plan, recipient)
    }

    #[test]
    fn signed_transaction_change_is_tracked_after_reopening_without_a_broadcast() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wallet.json");
        let (mut wallet, plan, recipient) = wallet_and_plan(&path);
        let change = plan.change_address.as_ref().unwrap();
        assert!(!wallet.tracked_addresses().unwrap().contains(change));

        let signed =
            super::build_sign_and_persist_transaction(&mut wallet, &plan, &recipient).unwrap();
        assert!(!signed.transaction_hex.is_empty());
        drop(wallet);
        let reopened = crate::local_wallet::load_and_unlock_wallet(&path, "test password").unwrap();
        assert!(reopened.tracked_addresses().unwrap().contains(change));
        assert_ne!(reopened.next_change_address().unwrap(), *change);
    }

    #[test]
    fn failed_change_save_does_not_return_a_signed_transaction() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wallet.json");
        let (mut wallet, plan, recipient) = wallet_and_plan(&path);
        // A directory at the wallet path forces the atomic replacement to fail.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        assert!(super::build_sign_and_persist_transaction(&mut wallet, &plan, &recipient).is_err());
        assert_eq!(
            wallet.next_change_address().unwrap(),
            *plan.change_address.as_ref().unwrap()
        );
    }

    #[test]
    fn changed_change_address_requires_a_new_review() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wallet.json");
        let (mut wallet, plan, recipient) = wallet_and_plan(&path);
        wallet.reserve_and_persist_next_change_address().unwrap();

        let error = match super::build_sign_and_persist_transaction(&mut wallet, &plan, &recipient)
        {
            Ok(_) => panic!("stale plan must not be signed"),
            Err(message) => message,
        };
        assert!(error.contains("Review the transaction again"));
    }

    #[test]
    fn north_amount_is_parsed_without_floating_point() {
        assert_eq!(parse_north("1.25000001"), Ok(125_000_001));
    }

    #[test]
    fn fee_warning_starts_at_ten_percent_of_the_payment() {
        assert!(!fee_is_large_share_of_payment(9_999, 100_000));
        assert!(fee_is_large_share_of_payment(10_000, 100_000));
        assert!(!fee_is_large_share_of_payment(10_000, 100_001));
    }
}
