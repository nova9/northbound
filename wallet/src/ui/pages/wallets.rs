use dioxus::prelude::*;

use crate::config::wallet_directory;
use crate::storage::catalog::{WalletFile, open_wallet_folder, read_wallet_files};
use crate::ui::components::{ErrorMessage, RemoveWalletDialog};
use crate::ui::routes::Route;
use crate::ui::state::AppContext;

#[component]
pub(crate) fn Wallets() -> Element {
    let mut context = use_context::<AppContext>();
    let navigator = use_navigator();
    let mut wallets =
        use_signal(|| wallet_directory().and_then(|folder| read_wallet_files(&folder)));
    let mut error = use_signal(|| None::<String>);
    let mut removing = use_signal(|| None::<WalletFile>);
    let selected = context.selected_wallet.read().clone();
    let unlocked = context.unlocked_wallet.read().is_some();
    let files = wallets.read().as_ref().ok().cloned().unwrap_or_default();

    rsx! {
        section { class: "screen workspace-screen wallets-screen",
            div { class: "page-heading",
                h1 { "Your wallets" }
                p { "Each wallet has its own recovery words and password. Switching locks the current wallet." }
            }
            div { class: "actions-row",
                button {
                    class: "button primary", r#type: "button",
                    onclick: move |_| {
                        context.unlocked_wallet.set(None);
                        context.selected_wallet.set(None);
                        navigator.push(Route::NewWallet);
                    },
                    "Create wallet"
                }
                button {
                    class: "button secondary", r#type: "button",
                    onclick: move |_| {
                        context.unlocked_wallet.set(None);
                        context.selected_wallet.set(None);
                        navigator.push(Route::Restore);
                    },
                    "Restore wallet"
                }
                button {
                    class: "button secondary", r#type: "button",
                    onclick: move |_| {
                        let result = wallet_directory().and_then(|folder| open_wallet_folder(&folder));
                        error.set(result.err());
                    },
                    "Open wallet folder"
                }
                button {
                    class: "text-button", r#type: "button",
                    onclick: move |_| {
                        wallets.set(wallet_directory().and_then(|folder| read_wallet_files(&folder)));
                        error.set(None);
                        removing.set(None);
                    },
                    "Reload list"
                }
            }
            div { class: "panel-scroll wallet-list",
                if let Err(message) = &*wallets.read() {
                    ErrorMessage { message: message.clone() }
                } else if files.is_empty() {
                    div { class: "empty-state",
                        h2 { "No wallets on this device" }
                        p { "Create a wallet or restore one using its twelve recovery words." }
                    }
                } else {
                    ul { class: "wallet-list-rows", aria_label: "Saved wallets",
                        for wallet in files {
                            li { class: "wallet-list-row", key: "{wallet.path.display()}",
                                div { class: "wallet-list-details",
                                    strong { "{wallet.name}" }
                                    span { class: "muted",
                                        if selected.as_ref() == Some(&wallet) && unlocked { "Unlocked" } else { "Locked" }
                                    }
                                }
                                button {
                                    class: "button secondary", r#type: "button",
                                    aria_label: "Open {wallet.name}",
                                    onclick: {
                                        let wallet = wallet.clone();
                                        move |_| {
                                            if context.selected_wallet.read().as_ref() == Some(&wallet)
                                                && context.unlocked_wallet.read().is_some() {
                                                navigator.replace(Route::Home);
                                                return;
                                            }
                                            context.unlocked_wallet.set(None);
                                            context.header_messages.write().clear();
                                            context.selected_wallet.set(Some(wallet.clone()));
                                            navigator.replace(Route::Unlock);
                                        }
                                    },
                                    if selected.as_ref() == Some(&wallet) && unlocked { "Open wallet" } else { "Unlock" }
                                }
                                button {
                                    class: "text-button wallet-remove-button", r#type: "button",
                                    aria_label: "Remove {wallet.name}",
                                    onclick: move |_| { removing.set(Some(wallet.clone())); error.set(None); },
                                    "Remove…"
                                }
                            }
                        }
                    }
                }
                if let Some(message) = error.read().as_deref() {
                    ErrorMessage { message: message.to_owned() }
                }
                if let Some(wallet) = removing() {
                    RemoveWalletDialog {
                        key: "{wallet.path.display()}",
                        wallet,
                        on_cancel: move |_| removing.set(None),
                        on_removed: move |_| {
                            removing.set(None);
                            wallets.set(wallet_directory().and_then(|folder| read_wallet_files(&folder)));
                        },
                    }
                }
            }
        }
    }
}
