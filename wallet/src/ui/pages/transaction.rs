use dioxus::prelude::*;

use crate::ui::components::TransactionHistoryView;
use crate::ui::state::AppContext;

#[component]
pub(crate) fn Transaction(txid: String) -> Element {
    let context = use_context::<AppContext>();
    let address_scanner = {
        let wallet = context.unlocked_wallet.read();

        match wallet.as_ref() {
            Some(wallet) => Ok(wallet.address_scanner()),
            None => Err("Unlock the wallet again to load this transaction.".to_owned()),
        }
    };

    let transactions = use_resource(move || {
        let address_scanner = address_scanner.clone();

        async move {
            let scanner = address_scanner?;
            let discovered = scanner.discover_addresses().await?;
            crate::api::fetch_transaction_history(discovered.tracked_addresses()).await
        }
    });

    rsx! {
        section { class: "screen history-screen",
            div { class: "transaction-heading",
                div {
                    h1 { "Transaction" }
                    p { class: "muted", "Review the recorded payment details." }
                }
            }

            match &*transactions.read() {
                None => rsx! {
                    div { class: "history-state", role: "status",
                        strong { "Loading transaction…" }
                    }
                },
                Some(Err(message)) => rsx! {
                    div { class: "history-state history-error", role: "alert",
                        strong { "Could not load transaction" }
                        p { "{message}" }
                    }
                },
                Some(Ok(transactions)) => {
                    let transaction = transactions
                        .iter()
                        .find(|transaction| transaction.transaction_id == txid)
                        .cloned();

                    match transaction {
                        Some(transaction) => rsx! {
                            TransactionHistoryView { transactions: vec![transaction] }
                        },
                        None => rsx! {
                            div { class: "history-state", role: "status",
                                strong { "Transaction not found" }
                                p { "This transaction is not part of the current wallet history." }
                            }
                        },
                    }
                },
            }
        }
    }
}
