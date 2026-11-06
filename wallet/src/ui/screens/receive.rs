use dioxus::prelude::*;

use crate::config::CURRENCY_TICKER;
use crate::ui::components::{AlertDialog, AlertTone};
use crate::ui::state::AppContext;

fn copy_address_to_clipboard(address: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Could not access the clipboard: {error}"))?;

    clipboard
        .set_text(address)
        .map_err(|error| format!("Could not copy the address: {error}"))
}

#[component]
pub(crate) fn ReceiveScreen() -> Element {
    let mut context = use_context::<AppContext>();
    let mut copied_address_index = use_signal(|| None::<u32>);
    let mut clipboard_error = use_signal(|| None::<String>);
    let mut address_creation_error = use_signal(|| None::<String>);
    let mut show_address_limit_alert = use_signal(|| false);
    let mut checking_address_limit = use_signal(|| false);

    let receiving_addresses = {
        let unlocked_wallet = context.unlocked_wallet.read();

        let Some(wallet) = unlocked_wallet.as_ref() else {
            return rsx! {
                p { "Unlock the wallet to view receiving addresses." }
            };
        };

        match wallet.receiving_addresses() {
            Ok(addresses) => addresses,
            Err(message) => {
                return rsx! {
                    p { class: "error-message", "{message}" }
                };
            }
        }
    };

    let current_address = receiving_addresses.last();
    let current_address_index = current_address.map(|address| address.index);

    rsx! {
        section { class: "screen receive-screen",
            div { class: "receive-heading",
                div {
                    h1 { "Receive {CURRENCY_TICKER}" }
                    p { class: "lede",
                        "Share this address to receive {CURRENCY_TICKER}. Create a fresh address for each payment."
                    }
                }
            }

            section {
                class: "address-history",
                aria_label: "Receiving address history",
                div { class: "section-heading history-heading",
                    div {
                        h2 { "Receiving addresses" }
                        p { "Every address shown here remains valid." }
                    }
                    div { class: "address-list-actions",
                        span { class: "address-count", "{receiving_addresses.len()} total" }
                        button {
                            class: "button primary compact-button",
                            r#type: "button",
                            disabled: checking_address_limit(),
                            onclick: move |_| {
                                checking_address_limit.set(true);
                                address_creation_error.set(None);
                                show_address_limit_alert.set(false);

                                spawn(async move {
                                    let addresses_to_check = {
                                        let unlocked_wallet = context.unlocked_wallet.read();
                                        let Some(wallet) = unlocked_wallet.as_ref() else {
                                            address_creation_error.set(Some("Wallet is locked".to_owned()));
                                            checking_address_limit.set(false);
                                            return;
                                        };

                                        match wallet.recent_receiving_addresses_for_allocation_check() {
                                            Ok(addresses) => addresses,
                                            Err(message) => {
                                                address_creation_error.set(Some(message));
                                                checking_address_limit.set(false);
                                                return;
                                            }
                                        }
                                    };

                                    let used_addresses = match crate::api::fetch_used_addresses(
                                            addresses_to_check.clone(),
                                        )
                                        .await
                                    {
                                        Ok(addresses) => addresses,
                                        Err(message) => {
                                            address_creation_error.set(Some(message));
                                            checking_address_limit.set(false);
                                            return;
                                        }
                                    };

                                    let mut unlocked_wallet = context.unlocked_wallet.write();
                                    let Some(wallet) = unlocked_wallet.as_mut() else {
                                        address_creation_error.set(Some("Wallet is locked".to_owned()));
                                        checking_address_limit.set(false);
                                        return;
                                    };

                                    match wallet
                                        .can_allocate_receiving_address(&addresses_to_check, &used_addresses)
                                    {
                                        Ok(true) => {
                                            match wallet.allocate_and_persist_next_receiving_address() {
                                                Ok(_) => address_creation_error.set(None),
                                                Err(message) => address_creation_error.set(Some(message)),
                                            }
                                        }
                                        Ok(false) => show_address_limit_alert.set(true),
                                        Err(message) => address_creation_error.set(Some(message)),
                                    }
                                    checking_address_limit.set(false);
                                });
                            },
                            if checking_address_limit() {
                                "Checking…"
                            } else if current_address.is_some() {
                                "New address"
                            } else {
                                "Create address"
                            }
                        }
                    }
                }

                if let Some(message) = address_creation_error() {
                    p {
                        class: "error-message address-list-error",
                        role: "alert",
                        "{message}"
                    }
                }
                if let Some(message) = clipboard_error() {
                    p {
                        class: "error-message address-list-error",
                        role: "alert",
                        "{message}"
                    }
                }

                div { class: "address-list",
                    for receiving_address in receiving_addresses.iter().rev() {
                        {
                            let address_index = receiving_address.index;
                            let address = receiving_address.address.clone();
                            let address_to_copy = address.clone();
                            let is_current = current_address_index == Some(address_index);

                            rsx! {
                                article { class: if is_current { "address-row current-address-row" } else { "address-row" },
                                    div { class: "address-row-meta",
                                        strong { "Address {address_index}" }
                                        if is_current {
                                            span { class: "address-status current", "Current" }
                                        } else {
                                            span { class: "address-status", "Earlier address" }
                                        }
                                    }

                                    code { "{address}" }

                                    button {
                                        class: "copy-link",
                                        r#type: "button",
                                        onclick: move |_| {
                                            match copy_address_to_clipboard(&address_to_copy) {
                                                Ok(()) => {
                                                    copied_address_index.set(Some(address_index));
                                                    clipboard_error.set(None);
                                                }
                                                Err(message) => {
                                                    copied_address_index.set(None);
                                                    clipboard_error.set(Some(message));
                                                }
                                            }
                                        },
                                        if copied_address_index() == Some(address_index) {
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

            if show_address_limit_alert() {
                AlertDialog {
                    title: "Unused address limit reached",
                    message: "You can have no more than 19 unused receiving addresses consecutively. Use one of your existing addresses to receive a payment. After it is used, you can create a new address.",
                    dismiss_label: "Got it",
                    tone: AlertTone::Information,
                    on_dismiss: move |_| show_address_limit_alert.set(false),
                }
            }
        }
    }
}
