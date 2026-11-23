use dioxus::prelude::*;

use crate::ui::components::TransactionHistoryView;
use crate::ui::state::AppContext;

#[component]
pub(crate) fn History() -> Element {
    let context = use_context::<AppContext>();
    let mut refresh_number = use_signal(|| 0_u64);

    let address_scanner = {
        let wallet = context.unlocked_wallet.read();

        match wallet.as_ref() {
            Some(wallet) => Ok(wallet.address_scanner()),
            None => Err("Unlock the wallet again to load transaction history.".to_owned()),
        }
    };

    let transaction_history = use_resource(move || {
        let _refresh_number = refresh_number();
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
                    h1 { "History" }
                    p { class: "muted", "Review payments sent and received by this wallet." }
                }
                button {
                    class: "button secondary history-refresh",
                    r#type: "button",
                    disabled: transaction_history.read().is_none(),
                    onclick: move |_| refresh_number += 1,
                    "Refresh"
                }
            }

            match &*transaction_history.read() {
                None => rsx! {
                    div { class: "history-state", role: "status",
                        strong { "Scanning for transactions…" }
                        p { "The first scan may take a little longer." }
                    }
                },
                Some(Err(message)) => rsx! {
                    div { class: "history-state history-error", role: "alert",
                        strong { "Could not load transaction history" }
                        p { "{message}" }
                    }
                },
                Some(Ok(transactions)) if transactions.is_empty() => rsx! {
                    div { class: "history-state",
                        strong { "No transactions yet" }
                        p { "Payments sent or received by this wallet will appear here." }
                    }
                },
                Some(Ok(transactions)) => rsx! {
                    TransactionHistoryView { transactions: transactions.clone() }
                },
            }
        }
    }
}
