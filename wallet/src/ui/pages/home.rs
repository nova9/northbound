use dioxus::prelude::*;

use crate::config::CURRENCY_TICKER;
use crate::local_wallet::{ReceivingAddress, calculate_spendable_balance};
use crate::ui::routes::Route;
use crate::ui::state::AppContext;

const ATOMIC_UNITS_PER_NORTH: u64 = 100_000_000;

struct OverviewData {
    current_address: ReceivingAddress,
    total_balance: Result<u64, String>,
    spendable_balance: Result<u64, String>,
}

#[component]
pub(crate) fn Home() -> Element {
    let context = use_context::<AppContext>();
    let navigator = use_navigator();
    let wallet_is_unlocked = context.unlocked_wallet.read().is_some();

    use_effect(move || {
        if wallet_is_unlocked {
            return;
        }

        let destination = if context.selected_wallet.read().is_some() {
            Route::Unlock
        } else {
            match crate::config::wallet_directory()
                .and_then(|directory| crate::wallet_catalog::read_wallet_files(&directory))
            {
                Ok(wallets) if wallets.is_empty() => Route::Welcome,
                _ => Route::Wallets,
            }
        };
        navigator.replace(destination);
    });

    let address_scanner = {
        let wallet = context.unlocked_wallet.read();
        wallet.as_ref().map(|wallet| wallet.address_scanner())
    };

    let Some(address_scanner) = address_scanner else {
        return rsx! {};
    };

    let wallet_path = context
        .selected_wallet
        .read()
        .as_ref()
        .map(|wallet| wallet.path.clone());

    let overview = use_resource(move || {
        let mut context = context;
        let scanner = address_scanner.clone();
        let wallet_path = wallet_path.clone();

        async move {
            let discovered = scanner.discover_addresses().await?;
            let current_address = discovered.active_receiving_address()?.clone();
            if let Some(wallet_path) = wallet_path.as_deref() {
                context.note_active_receiving_index(wallet_path, current_address.index);
            }
            let tracked_addresses = discovered.tracked_addresses();

            let total_balance =
                match crate::api::fetch_transaction_history(tracked_addresses.clone()).await {
                    Ok(transactions) => calculate_total_balance(&transactions),
                    Err(message) => Err(message),
                };
            let spendable_balance = match crate::api::fetch_utxos(tracked_addresses).await {
                Ok(utxos) => scanner
                    .verify_utxos(utxos, &discovered)
                    .and_then(|utxos| calculate_spendable_balance(&utxos)),
                Err(message) => Err(message),
            };

            Ok::<_, String>(OverviewData {
                current_address,
                total_balance,
                spendable_balance,
            })
        }
    });

    let overview_read = overview.read();

    rsx! {
        section { class: "screen overview-screen",
            header { class: "balance",
                div { class: "balance-heading",
                    h1 { "Total balance" }
                }
                div { class: "amount-row",
                    match &*overview_read {
                        None => rsx! {
                            strong { class: "amount", "Loading…" }
                        },
                        Some(Ok(data)) => match &data.total_balance {
                            Ok(amount) => rsx! {
                                strong { class: "amount", "{format_north(*amount)}" }
                                span { class: "currency", "{CURRENCY_TICKER}" }
                            },
                            Err(_) => rsx! {
                                strong { class: "amount", "Unavailable" }
                            },
                        },
                        Some(Err(_)) => rsx! {
                            strong { class: "amount", "Unavailable" }
                        },
                    }
                }
                p { class: "muted", "Includes pending transactions and immature mining rewards." }
                match &*overview_read {
                    Some(Err(message)) => rsx! {
                        p { class: "error-message", role: "alert", "Could not load wallet data: {message}" }
                    },
                    Some(Ok(data)) => {
                        if let Err(message) = &data.total_balance {
                            rsx! {
                                p { class: "error-message", role: "alert", "Could not load total balance: {message}" }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                    None => rsx! {},
                }
                dl { class: "spendable-balance",
                    dt { "Available to spend" }
                    dd {
                        match &*overview_read {
                            None => rsx! { span { class: "muted", "Loading…" } },
                            Some(Err(_)) => rsx! { span { class: "muted", "Unavailable" } },
                            Some(Ok(data)) => match &data.spendable_balance {
                                Ok(amount) => rsx! {
                                    strong { "{format_north(*amount)}" }
                                    span { class: "currency", "{CURRENCY_TICKER}" }
                                },
                                Err(message) => rsx! {
                                    p { class: "error-message", role: "alert", "Could not load spendable balance: {message}" }
                                },
                            },
                        }
                    }
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
                            onclick: move |_| {
                                navigator.push(Route::Receive);
                            },
                            span { class: "task-name", "Receive" }
                            span { class: "task-detail", "Show a payment address" }
                        }
                        button {
                            class: "task-button",
                            r#type: "button",
                            onclick: move |_| {
                                navigator.push(Route::Send);
                            },
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
                            onclick: move |_| {
                                navigator.push(Route::Receive);
                            },
                            "Manage addresses"
                        }
                    }
                    match &*overview_read {
                        None => rsx! {
                            p { class: "empty-copy", "Loading receiving address…" }
                        },
                        Some(Ok(data)) => rsx! {
                            code { class: "overview-address", "{data.current_address.address}" }
                            p { class: "address-caption",
                                "Address {data.current_address.index} · Unused · Advances automatically after payment"
                            }
                        },
                        Some(Err(message)) => rsx! {
                            p { class: "error-message", role: "alert", "Could not load receiving address: {message}" }
                        },
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

fn calculate_total_balance(transactions: &[shared::WalletTransaction]) -> Result<u64, String> {
    let mut total = 0_i128;
    for transaction in transactions {
        total = total
            .checked_add(i128::from(transaction.net_amount))
            .ok_or_else(|| "Transaction history balance is too large.".to_owned())?;
    }

    u64::try_from(total).map_err(|_| "Transaction history returned an invalid balance.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::WalletTransaction;

    #[test]
    fn total_balance_includes_pending_payment_and_its_fee() {
        let pending_send = WalletTransaction {
            transaction_id: "01".repeat(32),
            net_amount: -1_000_000_165,
            network_fee: Some(165),
            coinbase: false,
            block_height: None,
            confirmations: 0,
            timestamp: None,
            received_addresses: Vec::new(),
        };
        let mining_reward = WalletTransaction {
            transaction_id: "02".repeat(32),
            net_amount: 51_200_000_488,
            network_fee: None,
            coinbase: true,
            block_height: Some(26_073),
            confirmations: 802,
            timestamp: None,
            received_addresses: Vec::new(),
        };

        assert_eq!(
            calculate_total_balance(&[pending_send, mining_reward]),
            Ok(50_200_000_323)
        );
    }
}
