use dioxus::prelude::*;
use shared::WalletTransaction;

use crate::config::CURRENCY_TICKER;

const ATOMIC_UNITS_PER_NORTH: u64 = 100_000_000;

fn copy_transaction_id(transaction_id: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Could not access the clipboard: {error}"))?;

    clipboard
        .set_text(transaction_id)
        .map_err(|error| format!("Could not copy the transaction ID: {error}"))
}

fn format_north(amount: u64) -> String {
    let whole_units = amount / ATOMIC_UNITS_PER_NORTH;
    let fractional_units = amount % ATOMIC_UNITS_PER_NORTH;

    format!("{whole_units}.{fractional_units:08} {CURRENCY_TICKER}")
}

fn status_label(transaction: &WalletTransaction) -> &'static str {
    if transaction.confirmations == 0 {
        "Pending"
    } else {
        "Confirmed"
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
    let mut copied_transaction_id = use_signal(|| None::<String>);
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
    let status_class = if selected_transaction.confirmations == 0 {
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
                            let direction = if transaction_is_sent {
                                "Net sent"
                            } else if transaction.net_amount > 0 {
                                "Net received"
                            } else {
                                "No net change"
                            };
                            let amount = net_wallet_change(transaction);
                            let amount_class = if transaction_is_sent {
                                "transaction-value sent"
                            } else {
                                "transaction-value received"
                            };
                            let transaction_status = status_label(transaction);
                            let confirmation_summary = if transaction.confirmations == 0 {
                                "Waiting for confirmation".to_owned()
                            } else {
                                format_confirmation_count(transaction.confirmations)
                            };

                            rsx! {
                                button {
                                    class: "{row_class}",
                                    r#type: "button",
                                    aria_pressed: is_selected,
                                    onclick: move |_| {
                                        selected_index_signal.set(index);
                                        copied_transaction_id.set(None);
                                        copy_error.set(None);
                                    },
                                    div { class: "transaction-details",
                                        strong { "{direction}" }
                                        span { class: "transaction-meta",
                                            "{transaction_status} · {confirmation_summary}"
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
                        h2 { "Transaction details" }
                        span { class: "{status_class}", "{selected_status}" }
                    }
                    p { class: "transaction-status-copy",
                        if selected_transaction.confirmations == 0 {
                            "Seen by the node and waiting to be included in a block."
                        } else {
                            "Included in the chain and available to the wallet."
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
                        div {
                            dt { "Net wallet change" }
                            dd { "{net_wallet_change(selected_transaction)}" }
                        }

                    }

                    div { class: "transaction-id",
                        span { "Transaction ID" }
                        code { "{selected_transaction_id}" }
                        button {
                            class: "button secondary detail-copy",
                            r#type: "button",
                            onclick: move |_| {
                                match copy_transaction_id(&transaction_id_to_copy) {
                                    Ok(()) => {
                                        copied_transaction_id.set(Some(transaction_id_to_copy.clone()));
                                        copy_error.set(None);
                                    }
                                    Err(message) => copy_error.set(Some(message)),
                                }
                            },
                            if copied_transaction_id() == Some(selected_transaction_id) {
                                "Copied"
                            } else {
                                "Copy transaction ID"
                            }
                        }
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
    use super::{CURRENCY_TICKER, format_confirmation_count, format_north, net_wallet_change};
    use shared::WalletTransaction;

    #[test]
    fn net_change_includes_fees_once_and_formats_both_directions_and_zero() {
        let mut transaction = WalletTransaction {
            transaction_id: "ab".repeat(32),
            net_amount: -310_000_000,
            network_fee: Some(10_000_000),
            block_height: Some(120),
            confirmations: 1,
            timestamp: Some(1000),
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
}
