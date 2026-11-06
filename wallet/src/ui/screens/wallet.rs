use dioxus::prelude::*;

use crate::config::CURRENCY_TICKER;
use crate::local_wallet::calculate_spendable_balance;
use crate::ui::state::{AppContext, Route};

const ATOMIC_UNITS_PER_NORTH: u64 = 100_000_000;

#[component]
pub(crate) fn WalletScreen() -> Element {
    let mut context = use_context::<AppContext>();
    let wallet_state = {
        let wallet = context.unlocked_wallet.read();

        wallet.as_ref().map(|wallet| {
            let addresses = wallet.tracked_addresses();
            let current_address = wallet
                .receiving_addresses()
                .ok()
                .and_then(|mut addresses| addresses.pop());

            (addresses, current_address)
        })
    };

    let Some((addresses, current_address)) = wallet_state else {
        return rsx! {
            section { class: "screen narrow",
                h1 { "Wallet unavailable" }
                p { class: "lede", "Unlock the wallet again to continue." }
            }
        };
    };

    let utxos = use_resource(move || {
        let addresses = addresses.clone();

        async move {
            let addresses = addresses?;
            crate::api::fetch_utxos(addresses).await
        }
    });
    let balance = match &*utxos.read() {
        None => None,
        Some(Err(message)) => Some(Err(message.clone())),
        Some(Ok(utxos)) => {
            let wallet = context.unlocked_wallet.read();
            let verified = wallet
                .as_ref()
                .ok_or_else(|| "Unlock the wallet again to verify the balance.".to_owned())
                .and_then(|wallet| wallet.verify_utxos(utxos.clone()))
                .and_then(|utxos| calculate_spendable_balance(&utxos));

            Some(verified)
        }
    };

    rsx! {
        section { class: "screen overview-screen",
            header { class: "balance",
                div { class: "balance-heading",
                    h1 { "Available balance" }
                }
                div { class: "amount-row",
                    match &balance {
                        None => rsx! {
                            strong { class: "amount", "Loading…" }
                        },
                        Some(Ok(amount)) => rsx! {
                            strong { class: "amount", "{format_north(*amount)}" }
                            span { class: "currency", "{CURRENCY_TICKER}" }
                        },
                        Some(Err(_)) => rsx! {
                            strong { class: "amount", "Unavailable" }
                        },
                    }
                }
                if let Some(Err(message)) = &balance {
                    p { class: "error-message", role: "alert", "Could not load balance: {message}" }
                }
            }

            div { class: "overview-grid",
                section { class: "action-panel", aria_label: "Wallet actions",
                    h2 { "Move {CURRENCY_TICKER}" }
                    p { class: "section-copy",
                        "Choose what you want to do. Transaction details are always shown before signing."
                    }
                    div { class: "wallet-actions",
                        button {
                            class: "task-button primary-task",
                            r#type: "button",
                            onclick: move |_| context.route.set(Route::Receive),
                            span { class: "task-name", "Receive" }
                            span { class: "task-detail", "Show a payment address" }
                        }
                        button {
                            class: "task-button",
                            r#type: "button",
                            onclick: move |_| context.route.set(Route::Send),
                            span { class: "task-name", "Send" }
                            span { class: "task-detail", "Review before signing" }
                        }
                    }
                }

                section { class: "address-panel", aria_label: "Latest address",
                    div { class: "panel-heading",
                        h2 { "Latest address" }
                        button {
                            class: "text-button",
                            r#type: "button",
                            onclick: move |_| context.route.set(Route::Receive),
                            "Manage addresses"
                        }
                    }
                    if let Some(address) = current_address {
                        code { class: "overview-address", "{address.address}" }
                        p { class: "address-caption", "Address {address.index} · Ready to receive" }
                    } else {
                        p { class: "empty-copy", "Create your first address before receiving {CURRENCY_TICKER}." }
                    }
                }
            }

            aside { class: "service-notice", aria_label: "Wallet security status",
                strong { "Keys stay on this device" }
                span {
                    "The server receives public addresses only. It never receives your password, recovery phrase, or private keys."
                }
            }
        }
    }
}

fn format_north(amount: u64) -> String {
    let whole_units = amount / ATOMIC_UNITS_PER_NORTH;
    let fractional_units = amount % ATOMIC_UNITS_PER_NORTH;

    format!("{whole_units}.{fractional_units:08}")
}
