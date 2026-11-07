use dioxus::prelude::*;

use crate::ui::components::TransactionHistoryView;
use crate::ui::state::AppContext;

#[component]
pub(crate) fn Transaction(txid: String) -> Element {
    let context = use_context::<AppContext>();
    let addresses = {
        let wallet = context.unlocked_wallet.read();

        match wallet.as_ref() {
            Some(wallet) => wallet.tracked_addresses(),
            None => Err("Unlock the wallet again to load this transaction.".to_owned()),
        }
    };

    let transactions = use_resource(move || {
        let addresses = addresses.clone();

        async move {
            let addresses = addresses?;
            crate::api::fetch_transaction_history(addresses).await
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
