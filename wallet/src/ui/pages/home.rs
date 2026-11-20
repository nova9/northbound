use dioxus::prelude::*;

use crate::config::CURRENCY_TICKER;
use crate::local_wallet::calculate_spendable_balance;
use crate::ui::routes::Route;
use crate::ui::state::AppContext;

const ATOMIC_UNITS_PER_NORTH: u64 = 100_000_000;

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

    let wallet_state = {
        let wallet = context.unlocked_wallet.read();

        wallet.as_ref().map(|wallet| {
            let addresses = wallet.tracked_addresses();
            let current_address = wallet
                .receiving_addresses()
                .map(|mut addresses| addresses.pop());

            (addresses, current_address)
        })
    };

    let Some((addresses, current_address)) = wallet_state else {
        return rsx! {};
    };

    let history_addresses = addresses.clone();
    let total_balance = use_resource(move || {
        let addresses = history_addresses.clone();

        async move {
            let transactions = crate::api::fetch_transaction_history(addresses?).await?;
            calculate_total_balance(&transactions)
        }
    });
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
                    h1 { "Total balance" }
                }
                div { class: "amount-row",
                    match &*total_balance.read() {
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
                p { class: "muted", "Includes pending transactions and immature mining rewards." }
                if let Some(Err(message)) = &*total_balance.read() {
                    p { class: "error-message", role: "alert", "Could not load total balance: {message}" }
                }
                dl { class: "spendable-balance",
                    dt { "Available to spend" }
                    dd {
                        match &balance {
                            None => rsx! { span { class: "muted", "Loading…" } },
                            Some(Ok(amount)) => rsx! {
                                strong { "{format_north(*amount)}" }
                                span { class: "currency", "{CURRENCY_TICKER}" }
                            },
                            Some(Err(message)) => rsx! {
                                p { class: "error-message", role: "alert", "Could not load spendable balance: {message}" }
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
                    match current_address {
                        Ok(Some(address)) => rsx! {
                            code { class: "overview-address", "{address.address}" }
                            p { class: "address-caption", "Address {address.index} · Ready to receive" }
                        },
                        Ok(None) => rsx! {
                            p { class: "empty-copy", "Create your first address before receiving {CURRENCY_TICKER}." }
                        },
                        Err(message) => rsx! {
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
        };
        let mining_reward = WalletTransaction {
            transaction_id: "02".repeat(32),
            net_amount: 51_200_000_488,
            network_fee: None,
            coinbase: true,
            block_height: Some(26_073),
            confirmations: 802,
            timestamp: None,
        };

        assert_eq!(
            calculate_total_balance(&[pending_send, mining_reward]),
            Ok(50_200_000_323)
        );
    }
}
