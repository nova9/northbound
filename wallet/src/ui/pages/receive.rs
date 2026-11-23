use dioxus::prelude::*;

use crate::config::CURRENCY_TICKER;
use crate::ui::state::AppContext;

fn copy_address_to_clipboard(address: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Could not access the clipboard: {error}"))?;

    clipboard
        .set_text(address)
        .map_err(|error| format!("Could not copy the address: {error}"))
}

#[component]
pub(crate) fn Receive() -> Element {
    let context = use_context::<AppContext>();
    let mut copied_address_index = use_signal(|| None::<u32>);
    let mut clipboard_error = use_signal(|| None::<String>);
    let mut refresh_number = use_signal(|| 0_u64);

    let address_scanner = {
        let unlocked_wallet = context.unlocked_wallet.read();

        match unlocked_wallet.as_ref() {
            Some(wallet) => Ok(wallet.address_scanner()),
            None => Err("Unlock the wallet to view receiving addresses.".to_owned()),
        }
    };
    let wallet_path = context
        .selected_wallet
        .read()
        .as_ref()
        .map(|wallet| wallet.path.clone());

    let receiving_addresses = use_resource(move || {
        let _refresh_number = refresh_number();
        let mut context = context;
        let address_scanner = address_scanner.clone();
        let wallet_path = wallet_path.clone();

        async move {
            let scanner = address_scanner?;
            let addresses = scanner.discover_receiving_addresses().await?;
            if let (Some(wallet_path), Some(current)) = (wallet_path.as_deref(), addresses.last()) {
                context.note_active_receiving_index(wallet_path, current.index);
            }
            Ok::<_, String>(addresses)
        }
    });

    rsx! {
        section { class: "screen receive-screen",
            div { class: "transaction-heading receive-heading",
                div {
                    h1 { "Receive {CURRENCY_TICKER}" }
                    p { class: "lede",
                        "Share your current unused address to receive {CURRENCY_TICKER}."
                    }
                }
                button {
                    class: "button secondary",
                    r#type: "button",
                    disabled: receiving_addresses.read().is_none(),
                    onclick: move |_| refresh_number += 1,
                    "Refresh"
                }
            }

            if let Some(message) = clipboard_error() {
                p {
                    class: "error-message address-list-error",
                    role: "alert",
                    "{message}"
                }
            }

            match &*receiving_addresses.read() {
                None => rsx! {
                    div { class: "history-state", role: "status",
                        strong { "Loading receiving addresses…" }
                        p { "Checking the server for your latest unused address." }
                    }
                },
                Some(Err(message)) => rsx! {
                    p {
                        class: "error-message address-list-error",
                        role: "alert",
                        "{message}"
                    }
                },
                Some(Ok(addresses)) => {
                    let Some((current_address, earlier_addresses)) = addresses.split_last() else {
                        return rsx! {};
                    };
                    let current_index = current_address.index;
                    let current_value = current_address.address.clone();
                    let current_to_copy = current_value.clone();
                    let next_index_note = current_index
                        .checked_add(1)
                        .map(|next| {
                            format!(
                                "Once this address receives a payment, Address {next} will appear automatically."
                            )
                        })
                        .unwrap_or_else(|| {
                            "A new receive address appears automatically once this address receives a payment."
                                .to_owned()
                        });

                    rsx! {
                        section { class: "current-receive-card", aria_label: "Current receive address",
                            div { class: "current-receive-header",
                                div { class: "current-receive-title",
                                    strong { "Address {current_index}" }
                                    span { class: "address-status current", "Unused · Ready" }
                                }
                                button {
                                    class: "button primary compact-button",
                                    r#type: "button",
                                    onclick: move |_| {
                                        match copy_address_to_clipboard(&current_to_copy) {
                                            Ok(()) => {
                                                copied_address_index.set(Some(current_index));
                                                clipboard_error.set(None);
                                            }
                                            Err(message) => {
                                                copied_address_index.set(None);
                                                clipboard_error.set(Some(message));
                                            }
                                        }
                                    },
                                    if copied_address_index() == Some(current_index) {
                                        "Copied"
                                    } else {
                                        "Copy address"
                                    }
                                }
                            }

                            code { class: "current-receive-address", "{current_value}" }
                            p { class: "current-receive-note", "{next_index_note}" }
                        }

                        if !earlier_addresses.is_empty() {
                            section {
                                class: "address-history",
                                aria_label: "Earlier receiving addresses",
                                div { class: "section-heading history-heading",
                                    div {
                                        h2 { "Earlier addresses" }
                                        p { "Previous addresses remain valid, though using the current unused address is recommended." }
                                    }
                                    span { class: "address-count", "{earlier_addresses.len()} earlier" }
                                }

                                div { class: "address-list",
                                    for receiving_address in earlier_addresses.iter().rev() {
                                        {
                                            let address_index = receiving_address.index;
                                            let address = receiving_address.address.clone();
                                            let address_to_copy = address.clone();

                                            rsx! {
                                                article { class: "address-row",
                                                    div { class: "address-row-meta",
                                                        strong { "Address {address_index}" }
                                                        span { class: "address-status", "Earlier address" }
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
                        }
                    }
                }
            }
        }
    }
}
