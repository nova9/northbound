use dioxus::prelude::*;

use crate::amount::{format_north, format_optional_north_with_ticker, parse_north};
use crate::config::{CURRENCY_TICKER, WALLET_NETWORK};
use crate::crypto::p2qrh;
use crate::server::ServerClient;
use crate::transactions::{
    TransactionPlan, VerifiedUtxo, calculate_spendable_balance, create_transaction_plan,
    fee_is_large_share_of_payment,
};
use crate::ui::state::AppContext;

#[derive(Clone)]
enum SendState {
    Ready,
    Failed(String),
    Sending,
    Sent(String),
    Uncertain(String),
}

#[derive(Clone)]
struct SpendableFunds {
    verified_utxos: Vec<VerifiedUtxo>,
    change_address: String,
}

#[component]
pub(crate) fn Send() -> Element {
    let context = use_context::<AppContext>();
    let mut recipient = use_signal(String::new);
    let mut amount = use_signal(String::new);
    let mut validation_error = use_signal(|| None::<String>);
    let mut transaction_plan = use_signal(|| None::<TransactionPlan>);
    let mut send_state = use_signal(|| SendState::Ready);

    let address_scanner = {
        let wallet = context.unlocked_wallet.read();

        match wallet.as_ref() {
            Some(wallet) => Ok(wallet.address_scanner()),
            None => Err("Unlock the wallet again to load spendable funds.".to_owned()),
        }
    };

    let spendable_funds = use_resource(move || {
        let address_scanner = address_scanner.clone();

        async move {
            let scanner = address_scanner?;
            let server = ServerClient::from_configured_server()?;
            let discovered = scanner.discover_addresses(&server).await?;
            let utxos = server.fetch_utxos(discovered.tracked_addresses()).await?;
            let verified_utxos = scanner.verify_utxos(utxos, &discovered)?;
            let change_address = discovered.active_change_address()?.to_owned();

            Ok::<_, String>(SpendableFunds {
                verified_utxos,
                change_address,
            })
        }
    });
    let fee_estimate = use_resource(|| async {
        let server = ServerClient::from_configured_server()?;
        server.fetch_fee_estimate().await
    });

    let spendable_funds_state = spendable_funds.read().clone();
    let available_balance = spendable_funds_state
        .as_ref()
        .and_then(|funds| funds.as_ref().ok())
        .and_then(|funds| calculate_spendable_balance(&funds.verified_utxos).ok());
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
                            &spendable_funds_state,
                            atomic_units_per_kvbyte,
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
                            match &spendable_funds_state {
                                None => "Loading spendable balance…".to_owned(),
                                Some(Err(_)) => "Spendable balance unavailable".to_owned(),
                                Some(Ok(_)) => match available_balance {
                                    Some(balance) => format!("Available: {} {CURRENCY_TICKER}", format_north(balance)),
                                    None => "Spendable balance unavailable".to_owned(),
                                },
                            }
                        }
                    }

                    if let Some(Err(message)) = &spendable_funds_state {
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
                        span { "{format_optional_north_with_ticker(reviewed_fee)}" }
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
                            dd { "{format_optional_north_with_ticker(reviewed_plan.as_ref().map(|plan| plan.recipient_amount).or(entered_amount))}" }
                        }
                        div {
                            dt { "Estimated fee" }
                            dd { "{format_optional_north_with_ticker(reviewed_fee)}" }
                        }
                        div { class: "send-total",
                            dt { "Total" }
                            dd { "{format_optional_north_with_ticker(reviewed_total)}" }
                        }
                        div {
                            dt { "Remaining balance" }
                            dd { "{format_optional_north_with_ticker(reviewed_remaining)}" }
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
                                                let wallet = context.unlocked_wallet.read();
                                                match wallet.as_ref() {
                                                    Some(wallet) => wallet.build_and_sign_transaction(&plan, &recipient()),
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
                                                let server = match ServerClient::from_configured_server() {
                                                    Ok(server) => server,
                                                    Err(message) => {
                                                        send_state.set(SendState::Failed(message));
                                                        return;
                                                    }
                                                };
                                                let expected_transaction_id = signed_transaction.transaction_id;
                                                match server.broadcast_signed_transaction(signed_transaction.transaction_hex).await {
                                                    Ok(transaction_id) => send_state.set(SendState::Sent(transaction_id)),
                                                    Err(broadcast_error) => {
                                                        let status = server
                                                            .fetch_transaction_status(expected_transaction_id.clone())
                                                            .await;
                                                        send_state.set(send_state_after_status_check(
                                                            expected_transaction_id, broadcast_error, status,
                                                        ));
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

fn send_state_after_status_check(
    transaction_id: String,
    broadcast_error: String,
    status: Result<bool, String>,
) -> SendState {
    match status {
        Ok(true) => SendState::Sent(transaction_id),
        Ok(false) => SendState::Uncertain(broadcast_error),
        Err(status_error) => SendState::Uncertain(format!(
            "{broadcast_error} Status check also failed: {status_error}"
        )),
    }
}

fn prepare_transaction_plan(
    recipient: &str,
    amount: &str,
    spendable_funds_state: &Option<Result<SpendableFunds, String>>,
    atomic_units_per_kvbyte: Option<u64>,
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

    let funds = spendable_funds_state
        .as_ref()
        .ok_or_else(|| "Wait for the spendable balance to finish loading.".to_owned())?
        .as_ref()
        .map_err(Clone::clone)?;

    create_transaction_plan(
        funds.verified_utxos.clone(),
        amount,
        atomic_units_per_kvbyte,
        funds.change_address.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::{SendState, send_state_after_status_check};

    #[test]
    fn broadcast_errors_require_status_checks_and_preserve_uncertainty_and_reasons() {
        for broadcast_error in [
            "Could not send the transaction: dust",
            "could not contact wallet server",
        ] {
            let sent =
                send_state_after_status_check("txid".into(), broadcast_error.into(), Ok(true));
            assert!(matches!(sent, SendState::Sent(id) if id == "txid"));
            let unknown =
                send_state_after_status_check("txid".into(), broadcast_error.into(), Ok(false));
            assert!(matches!(unknown, SendState::Uncertain(message) if message == broadcast_error));
            let unavailable = send_state_after_status_check(
                "txid".into(),
                broadcast_error.into(),
                Err("node unavailable".into()),
            );
            assert!(matches!(unavailable, SendState::Uncertain(message)
                if message.contains(broadcast_error) && message.contains("node unavailable")));
        }
    }
}
