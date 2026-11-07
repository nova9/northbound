use dioxus::prelude::*;

use crate::config::WALLET_DESCRIPTION;
use crate::config::WALLET_NAME;
use crate::ui::components::Logo;
use crate::ui::routes::Route;

#[component]
pub(crate) fn Welcome() -> Element {
    let navigator = use_navigator();

    rsx! {
        section { class: "screen welcome-screen",
            div { class: "welcome-layout",
                div { class: "welcome-introduction",
                    Logo { size: "64".to_string() }
                    h1 { "Welcome to {WALLET_NAME}" }
                    p { class: "welcome-description", "{WALLET_DESCRIPTION}" }
                    h2 { "Your wallet starts here" }
                    p {
                        "Create a wallet, back up your recovery words, and choose a password to protect it on this device."
                    }
                    button {
                        class: "button primary",
                        onclick: move |_| {
                            navigator.push(Route::NewWallet);
                        },
                        "Create new wallet"
                    }
                    div { class: "welcome-restore",
                        h2 { "Already have a wallet?" }
                        p { "Bring it to this device using your 12-word recovery phrase." }
                        button {
                            class: "button secondary",
                            onclick: move |_| {
                                navigator.push(Route::Restore);
                            },
                            "Restore existing wallet"
                        }
                    }
                }
                aside { class: "welcome-backup",
                    h2 { "You hold the keys" }
                    p {
                        "Your recovery phrase is the backup for your wallet. Keep it somewhere private and safe."
                    }
                    dl {
                        div {
                            dt { "Write down all 12 words" }
                            dd { "Keep them in order. You will confirm them during setup." }
                        }
                        div {
                            dt { "Keep your backup offline" }
                            dd { "Anyone with these words can spend your funds. Never share them." }
                        }
                        div {
                            dt { "Understand your password" }
                            dd {
                                "It protects the wallet on this device. It cannot replace your recovery phrase."
                            }
                        }
                    }
                }
            }
            p { class: "welcome-footer",
                "Keys are created and transactions are signed on your device."
            }
        }
    }
}
