use bitcoin::consensus::encode;
use dioxus::prelude::*;

use crate::config::{CURRENCY_TICKER, WALLET_NETWORK};
use crate::local_wallet::{VerifiedUtxo, calculate_spendable_balance};
use crate::p2qrh;
use crate::transaction_plan::{TransactionPlan, create_transaction_plan};
use crate::ui::state::AppContext;
use crate::unsigned_transaction::build_unsigned_transaction;

const ATOMIC_UNITS_PER_NORTH: u64 = 100_000_000;

#[derive(Clone)]
enum NodeValidationState {
    NotRequested,
    Checking,
    Accepted(SignedTransaction),
    Rejected(String),
    Broadcasting,
    Broadcast(String),
    BroadcastUncertain(String),
}

#[derive(Clone)]
struct SignedTransaction {
    transaction_hex: String,
    transaction_id: String,
}

#[component]
pub(crate) fn SendScreen() -> Element {
    let context = use_context::<AppContext>();
    let mut recipient = use_signal(String::new);
    let mut amount = use_signal(String::new);
    let mut validation_error = use_signal(|| None::<String>);
    let mut transaction_plan = use_signal(|| None::<TransactionPlan>);
    let mut node_validation = use_signal(|| NodeValidationState::NotRequested);

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
    let fee_rate = fee_estimate_state
        .as_ref()
        .and_then(|estimate| estimate.as_ref().ok())
        .map(|estimate| estimate.atomic_units_per_vbyte);

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

                        let result = prepare_transaction_plan(
                            &recipient(),
                            &amount(),
                            &verified_utxos_state,
                            fee_rate,
                            &context,
                        );

                        match result {
                            Ok(plan) => {
                                transaction_plan.set(Some(plan));
                                node_validation.set(NodeValidationState::NotRequested);
                                validation_error.set(None);
                            }
                            Err(message) => {
                                transaction_plan.set(None);
                                node_validation.set(NodeValidationState::NotRequested);
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
                            spellcheck: "false",
                            oninput: move |event| {
                                recipient.set(event.value());
                                transaction_plan.set(None);
                                node_validation.set(NodeValidationState::NotRequested);
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
                                oninput: move |event| {
                                    amount.set(event.value());
                                    transaction_plan.set(None);
                                    node_validation.set(NodeValidationState::NotRequested);
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
                                        "Node estimate · {} atomic units/vB · {} block target",
                                        estimate.atomic_units_per_vbyte,
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
                        disabled: available_balance.is_none() || fee_rate.is_none(),
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
                            if matches!(node_validation(), NodeValidationState::NotRequested) {
                                p { "Review the amount and recipient before signing." }
                            }

                            match node_validation() {
                                NodeValidationState::NotRequested => rsx! {
                                    button {
                                        class: "button secondary",
                                        r#type: "button",
                                        onclick: move |_| {
                                            let result = build_and_sign_transaction(
                                                &context,
                                                &plan,
                                                &recipient(),
                                            );

                                            match result {
                                                Ok(signed_transaction) => {
                                                    node_validation.set(NodeValidationState::Checking);
                                                    let transaction_to_validate = signed_transaction.transaction_hex.clone();

                                                    spawn(async move {
                                                        let result = crate::api::validate_signed_transaction(transaction_to_validate).await;

                                                        match result {
                                                            Ok(result) if result.allowed => {
                                                                node_validation.set(NodeValidationState::Accepted(signed_transaction));
                                                            }
                                                            Ok(result) => {
                                                                let reason = result.rejection_reason.unwrap_or_else(|| {
                                                                    "The node rejected the transaction without a reason.".to_owned()
                                                                });
                                                                node_validation.set(NodeValidationState::Rejected(reason));
                                                            }
                                                            Err(message) => {
                                                                node_validation.set(NodeValidationState::Rejected(message));
                                                            }
                                                        }
                                                    });
                                                }
                                                Err(message) => {
                                                    node_validation.set(NodeValidationState::Rejected(message));
                                                }
                                            }
                                        },
                                        "Validate signed transaction"
                                    }
                                },
                                NodeValidationState::Checking => rsx! {
                                    button { class: "button secondary", r#type: "button", disabled: true,
                                        "Checking with node…"
                                    }
                                },
                                NodeValidationState::Accepted(signed_transaction) => rsx! {
                                    div { role: "status",
                                        p { class: "success-message",
                                            "Accepted by the node and ready for broadcast. It has not been sent."
                                        }
                                        p { "Broadcasting will send {format_north(plan.recipient_amount)} {CURRENCY_TICKER} to the recipient." }
                                        button {
                                            class: "button primary",
                                            r#type: "button",
                                            onclick: move |_| {
                                                match reserve_planned_change_address(context, &plan) {
                                                    Ok(()) => {
                                                        node_validation.set(NodeValidationState::Broadcasting);
                                                        let transaction_to_broadcast = signed_transaction.transaction_hex.clone();
                                                        let expected_transaction_id = signed_transaction.transaction_id.clone();

                                                        spawn(async move {
                                                            match crate::api::broadcast_signed_transaction(transaction_to_broadcast).await {
                                                                Ok(transaction_id) => {
                                                                    node_validation.set(NodeValidationState::Broadcast(transaction_id));
                                                                }
                                                                Err(broadcast_error) => {
                                                                    let status = crate::api::fetch_transaction_status(
                                                                        expected_transaction_id.clone(),
                                                                    ).await;

                                                                    match status {
                                                                        Ok(true) => node_validation.set(
                                                                            NodeValidationState::Broadcast(expected_transaction_id),
                                                                        ),
                                                                        Ok(false) => node_validation.set(
                                                                            NodeValidationState::BroadcastUncertain(broadcast_error),
                                                                        ),
                                                                        Err(status_error) => node_validation.set(
                                                                            NodeValidationState::BroadcastUncertain(format!(
                                                                                "{broadcast_error} Status check also failed: {status_error}"
                                                                            )),
                                                                        ),
                                                                    }
                                                                }
                                                            }
                                                        });
                                                    }
                                                    Err(message) => {
                                                        node_validation.set(NodeValidationState::Rejected(message));
                                                    }
                                                }
                                            },
                                            "Broadcast transaction"
                                        }
                                    }
                                },
                                NodeValidationState::Rejected(message) => rsx! {
                                    p { class: "error-message", role: "alert",
                                        "Node rejected the transaction: {message}"
                                    }
                                },
                                NodeValidationState::Broadcasting => rsx! {
                                    button { class: "button primary", r#type: "button", disabled: true,
                                        "Broadcasting…"
                                    }
                                },
                                NodeValidationState::Broadcast(transaction_id) => rsx! {
                                    div { role: "status",
                                        p { class: "success-message", "Transaction sent." }
                                        p { class: "transaction-id", "Transaction ID: {transaction_id}" }
                                    }
                                },
                                NodeValidationState::BroadcastUncertain(message) => rsx! {
                                    p { class: "error-message", role: "alert",
                                        "The broadcast result is uncertain: {message} Check History before trying anything else."
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

fn build_and_sign_transaction(
    context: &AppContext,
    plan: &TransactionPlan,
    recipient: &str,
) -> Result<SignedTransaction, String> {
    let mut transaction = build_unsigned_transaction(plan, recipient)?;
    let wallet = context.unlocked_wallet.read();
    let wallet = wallet
        .as_ref()
        .ok_or_else(|| "Unlock the wallet again to sign the transaction.".to_owned())?;

    wallet.sign_p2qrh_transaction(&mut transaction, &plan.inputs)?;

    Ok(SignedTransaction {
        transaction_id: transaction.compute_txid().to_string(),
        transaction_hex: encode::serialize_hex(&transaction),
    })
}

fn reserve_planned_change_address(
    mut context: AppContext,
    plan: &TransactionPlan,
) -> Result<(), String> {
    if plan.change == 0 {
        return Ok(());
    }

    let planned_change_address = plan
        .change_address
        .as_deref()
        .ok_or_else(|| "The transaction plan is missing its change address.".to_owned())?;
    let mut wallet = context.unlocked_wallet.write();
    let wallet = wallet
        .as_mut()
        .ok_or_else(|| "Unlock the wallet again before broadcasting.".to_owned())?;
    let next_change_address = wallet.next_change_address()?;

    if next_change_address != planned_change_address {
        return Err("The wallet change address changed. Review the transaction again.".to_owned());
    }

    wallet.reserve_and_persist_next_change_address()?;

    Ok(())
}

fn prepare_transaction_plan(
    recipient: &str,
    amount: &str,
    verified_utxos_state: &Option<Result<Vec<VerifiedUtxo>, String>>,
    fee_rate: Option<u64>,
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

    let fee_rate =
        fee_rate.ok_or_else(|| "Wait for the network fee estimate to load.".to_owned())?;

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

    create_transaction_plan(verified_utxos, amount, fee_rate, change_address)
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

#[cfg(test)]
mod tests {
    use super::parse_north;

    #[test]
    fn north_amount_is_parsed_without_floating_point() {
        assert_eq!(parse_north("1.25000001"), Ok(125_000_001));
    }
}
