use dioxus::prelude::*;
use shared::{COINBASE_MATURITY_CONFIRMATIONS, WalletTransaction};

use crate::config::CURRENCY_TICKER;

const ATOMIC_UNITS_PER_NORTH: u64 = 100_000_000;

fn copy_text(value: &str, label: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Could not access the clipboard: {error}"))?;

    clipboard
        .set_text(value)
        .map_err(|error| format!("Could not copy {label}: {error}"))
}

fn format_north(amount: u64) -> String {
    let whole_units = amount / ATOMIC_UNITS_PER_NORTH;
    let fractional_units = amount % ATOMIC_UNITS_PER_NORTH;

    format!("{whole_units}.{fractional_units:08} {CURRENCY_TICKER}")
}

fn status_label(transaction: &WalletTransaction) -> &'static str {
    if is_immature_mining_reward(transaction) {
        "Immature"
    } else if transaction.confirmations == 0 {
        "Pending"
    } else {
        "Confirmed"
    }
}

fn is_immature_mining_reward(transaction: &WalletTransaction) -> bool {
    transaction.coinbase && transaction.confirmations < COINBASE_MATURITY_CONFIRMATIONS
}

fn is_receive_item(transaction: &WalletTransaction) -> bool {
    transaction.coinbase || transaction.net_amount > 0
}

fn transaction_label(transaction: &WalletTransaction) -> &'static str {
    if transaction.coinbase {
        "Mining reward"
    } else if transaction.net_amount < 0 {
        "Net sent"
    } else if transaction.net_amount > 0 {
        "Net received"
    } else {
        "No net change"
    }
}

fn format_confirmation_count(confirmations: u64) -> String {
    if confirmations == 1 {
        "1 confirmation".to_owned()
    } else {
        format!("{confirmations} confirmations")
    }
}

fn block_height(transaction: &WalletTransaction) -> String {
    match transaction.block_height {
        Some(height) => height.to_string(),
        None => "Not mined".to_owned(),
    }
}

fn net_wallet_change(transaction: &WalletTransaction) -> String {
    let sign = if transaction.net_amount < 0 {
        "−"
    } else if transaction.net_amount > 0 {
        "+"
    } else {
        ""
    };
    format!(
        "{sign}{}",
        format_north(transaction.net_amount.unsigned_abs())
    )
}

#[component]
pub(crate) fn TransactionHistoryView(transactions: Vec<WalletTransaction>) -> Element {
    let mut selected_index_signal = use_signal(|| 0_usize);
    let mut copied_value = use_signal(|| None::<String>);
    let mut copy_error = use_signal(|| None::<String>);

    let selected_index = selected_index_signal().min(transactions.len() - 1);
    let selected_transaction = &transactions[selected_index];
    let selected_transaction_id = selected_transaction.transaction_id.clone();
    let transaction_id_to_copy = selected_transaction_id.clone();
    let selected_status = status_label(selected_transaction);
    let selected_fee = selected_transaction
        .network_fee
        .map(format_north)
        .unwrap_or_else(|| "Not applicable".to_owned());
    let status_class = if is_immature_mining_reward(selected_transaction) {
        "transaction-status immature"
    } else if selected_transaction.confirmations == 0 {
        "transaction-status pending"
    } else {
        "transaction-status confirmed"
    };

    rsx! {
        section { class: "transaction-history", aria_label: "Transaction history",
            div { class: "transaction-browser",
                div { class: "transaction-list",
                    for (index, transaction) in transactions.iter().enumerate() {
                        {
                            let is_selected = selected_index == index;
                            let row_class = if is_selected {
                                "transaction-row selected"
                            } else {
                                "transaction-row"
                            };
                            let transaction_is_sent = transaction.net_amount < 0;
                            let direction = transaction_label(transaction);
                            let amount = net_wallet_change(transaction);
                            let amount_class = if transaction_is_sent {
                                "transaction-value sent"
                            } else {
                                "transaction-value received"
                            };
                            let transaction_status = status_label(transaction);
                            rsx! {
                                button {
                                    class: "{row_class}",
                                    r#type: "button",
                                    aria_pressed: is_selected,
                                    onclick: move |_| {
                                        selected_index_signal.set(index);
                                        copied_value.set(None);
                                        copy_error.set(None);
                                    },
                                    div { class: "transaction-details",
                                        strong { "{direction}" }
                                        span { class: "transaction-meta",
                                            "{transaction_status}"
                                        }
                                    }
                                    div { class: "transaction-numbers",
                                        strong { class: "{amount_class}", "{amount}" }
                                        if let Some(height) = transaction.block_height {
                                            span { class: "transaction-time", "Block {height}" }
                                        } else {
                                            span { class: "transaction-time", "Mempool" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                section { class: "transaction-detail", aria_label: "Transaction details",
                    div { class: "transaction-detail-heading",
                        h2 { "{transaction_label(selected_transaction)}" }
                        span { class: "{status_class}", "{selected_status}" }
                    }
                    p { class: "transaction-detail-amount", "{net_wallet_change(selected_transaction)}" }
                    p { class: "transaction-status-copy",
                        if is_immature_mining_reward(selected_transaction) {
                            "Mining rewards become available at {COINBASE_MATURITY_CONFIRMATIONS} confirmations. This reward is not included in your available balance yet."
                        } else if selected_transaction.confirmations == 0 {
                            "Seen by the node and waiting to be included in a block."
                        } else if selected_transaction.coinbase {
                            "This mining reward has matured."
                        } else {
                            "Included in the chain."
                        }
                    }

                    dl { class: "transaction-facts",
                        div {
                            dt { "Confirmations" }
                            dd { "{format_confirmation_count(selected_transaction.confirmations)}" }
                        }
                        div {
                            dt { "Block height" }
                            dd { "{block_height(selected_transaction)}" }
                        }
                        div {
                            dt { "Total network fee" }
                            dd {
                                "{selected_fee}"
                            }
                        }
                        if is_receive_item(selected_transaction)
                            && !selected_transaction.received_addresses.is_empty()
                        {
                            if selected_transaction.received_addresses.len() == 1 {
                                {
                                    let receive_address = selected_transaction.received_addresses[0].clone();
                                    let receive_address_to_copy = receive_address.clone();

                                    rsx! {
                                        div {
                                            dt { class: "transaction-field-heading",
                                                span { "Receive address" }
                                                button {
                                                    class: "copy-link",
                                                    r#type: "button",
                                                    onclick: move |_| {
                                                        match copy_text(&receive_address_to_copy, "the receive address") {
                                                            Ok(()) => {
                                                                copied_value.set(Some(receive_address_to_copy.clone()));
                                                                copy_error.set(None);
                                                            }
                                                            Err(message) => copy_error.set(Some(message)),
                                                        }
                                                    },
                                                    if copied_value().as_deref() == Some(receive_address.as_str()) {
                                                        "Copied"
                                                    } else {
                                                        "Copy"
                                                    }
                                                }
                                            }
                                            dd { class: "transaction-addresses",
                                                code { "{receive_address}" }
                                            }
                                        }
                                    }
                                }
                            } else {
                                div {
                                    dt { "Receive addresses" }
                                    dd { class: "transaction-addresses",
                                        for address in &selected_transaction.received_addresses {
                                            {
                                                let receive_address = address.clone();
                                                let receive_address_to_copy = receive_address.clone();

                                                rsx! {
                                                    div { class: "transaction-address-item",
                                                        code { "{receive_address}" }
                                                        button {
                                                            class: "copy-link",
                                                            r#type: "button",
                                                            onclick: move |_| {
                                                                match copy_text(&receive_address_to_copy, "the receive address") {
                                                                    Ok(()) => {
                                                                        copied_value.set(Some(receive_address_to_copy.clone()));
                                                                        copy_error.set(None);
                                                                    }
                                                                    Err(message) => copy_error.set(Some(message)),
                                                                }
                                                            },
                                                            if copied_value().as_deref() == Some(receive_address.as_str()) {
                                                                "Copied"
                                                            } else {
                                                                "Copy"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    div { class: "transaction-id",
                        div { class: "transaction-field-heading",
                            span { "Transaction ID" }
                            button {
                                class: "copy-link",
                                r#type: "button",
                                onclick: move |_| {
                                    match copy_text(&transaction_id_to_copy, "the transaction ID") {
                                        Ok(()) => {
                                            copied_value.set(Some(transaction_id_to_copy.clone()));
                                            copy_error.set(None);
                                        }
                                        Err(message) => copy_error.set(Some(message)),
                                    }
                                },
                                if copied_value().as_deref() == Some(selected_transaction_id.as_str()) {
                                    "Copied"
                                } else {
                                    "Copy"
                                }
                            }
                        }
                        code { "{selected_transaction_id}" }
                    }

                    if let Some(message) = copy_error() {
                        p { class: "error-message transaction-copy-error", "{message}" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CURRENCY_TICKER, format_confirmation_count, format_north, is_receive_item,
        net_wallet_change, status_label, transaction_label,
    };
    use shared::WalletTransaction;

    #[test]
    fn net_change_includes_fees_once_and_formats_both_directions_and_zero() {
        let mut transaction = WalletTransaction {
            transaction_id: "ab".repeat(32),
            net_amount: -310_000_000,
            network_fee: Some(10_000_000),
            coinbase: false,
            block_height: Some(120),
            confirmations: 1,
            timestamp: Some(1000),
            received_addresses: Vec::new(),
        };
        assert_eq!(
            net_wallet_change(&transaction),
            format!("−3.10000000 {CURRENCY_TICKER}")
        );
        transaction.net_amount = 690_000_000;
        assert_eq!(
            net_wallet_change(&transaction),
            format!("+6.90000000 {CURRENCY_TICKER}")
        );
        transaction.net_amount = 0;
        assert_eq!(
            net_wallet_change(&transaction),
            format!("0.00000000 {CURRENCY_TICKER}")
        );
        transaction.net_amount = -10_000_000;
        assert_eq!(
            net_wallet_change(&transaction),
            format!("−0.10000000 {CURRENCY_TICKER}")
        );
        transaction.net_amount = i64::MIN;
        assert!(net_wallet_change(&transaction).starts_with('−'));
    }

    #[test]
    fn atomic_units_are_formatted_as_north() {
        assert_eq!(
            format_north(1_025_000_001),
            format!("10.25000001 {CURRENCY_TICKER}")
        );
    }

    #[test]
    fn one_confirmation_uses_singular_grammar() {
        assert_eq!(format_confirmation_count(1), "1 confirmation");
        assert_eq!(format_confirmation_count(2), "2 confirmations");
    }

    #[test]
    fn receive_items_include_received_and_coinbase_transactions_only() {
        let mut transaction = WalletTransaction {
            transaction_id: "ab".repeat(32),
            net_amount: 100_000_000,
            network_fee: Some(165),
            coinbase: false,
            block_height: Some(120),
            confirmations: 1,
            timestamp: Some(1000),
            received_addresses: Vec::new(),
        };

        assert!(is_receive_item(&transaction));

        transaction.net_amount = 0;
        assert!(!is_receive_item(&transaction));

        transaction.net_amount = -100_000_000;
        assert!(!is_receive_item(&transaction));

        transaction.coinbase = true;
        transaction.net_amount = 51_200_000_000;
        assert!(is_receive_item(&transaction));
    }

    #[test]
    fn mining_rewards_are_identified_and_immature_until_the_balance_accepts_them() {
        let mut transaction = WalletTransaction {
            transaction_id: "ab".repeat(32),
            net_amount: 51_200_000_000,
            network_fee: None,
            coinbase: true,
            block_height: Some(120),
            confirmations: 68,
            timestamp: Some(1000),
            received_addresses: Vec::new(),
        };
        assert_eq!(transaction_label(&transaction), "Mining reward");
        assert_eq!(status_label(&transaction), "Immature");
        transaction.confirmations = 99;
        assert_eq!(status_label(&transaction), "Immature");
        transaction.confirmations = 100;
        assert_eq!(status_label(&transaction), "Confirmed");
        assert_eq!(transaction_label(&transaction), "Mining reward");

        transaction.coinbase = false;
        transaction.confirmations = 1;
        assert_eq!(status_label(&transaction), "Confirmed");
        assert_eq!(transaction_label(&transaction), "Net received");
        transaction.confirmations = 0;
        assert_eq!(status_label(&transaction), "Pending");
    }
}
