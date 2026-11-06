use dioxus::prelude::*;

use crate::keys;
use crate::local_wallet::save_new_wallet_and_unlock;
use crate::ui::components::{ErrorMessage, Field, RecoveryWords};
use crate::ui::sensitive_text::SensitiveText;
use crate::ui::state::{AppContext, Route};

#[component]
pub(crate) fn NewWalletScreen() -> Element {
    let mut context = use_context::<AppContext>();
    let pending_wallet = use_signal(keys::generate_new_wallet);
    let mut entered_phrase = use_signal(SensitiveText::default);
    let mut password = use_signal(SensitiveText::default);
    let mut password_confirmation = use_signal(SensitiveText::default);
    let mut error = use_signal(|| None::<String>);

    let recovery_phrase = pending_wallet
        .read()
        .as_ref()
        .map(|wallet| wallet.recovery_phrase().to_owned())
        .ok();

    let generation_error = pending_wallet.read().as_ref().err().map(ToOwned::to_owned);

    rsx! {
        section { class: "screen new-wallet-screen",
            header { class: "new-wallet-heading",
                h1 { "Back up your wallet" }
                p { class: "lede",
                    "Write these words down in order. They are the only backup that can restore your wallet."
                }
            }

            if let Some(phrase) = recovery_phrase {
                div { class: "wallet-backup-layout",
                    section {
                        class: "recovery-phrase-panel",
                        aria_label: "Recovery phrase",
                        h2 { "Recovery phrase" }
                        p { "Keep these words private and offline." }
                        RecoveryWords { phrase }
                    }

                    section {
                        class: "wallet-confirmation-panel",
                        aria_label: "Confirm wallet backup",
                        h2 { "Confirm and protect" }
                        p { "Re-enter the phrase, then choose a password for this device." }
                        div { class: "form-stack",
                            Field {
                                label: "Recovery phrase",
                                autocomplete: "off",
                                input_type: "text",
                                placeholder: "Twelve recovery words",
                                value: entered_phrase.read().to_string(),
                                oninput: move |value| {
                                    entered_phrase.write().replace(value);
                                    error.set(None);
                                },
                            }
                            Field {
                                label: "Wallet password",
                                input_type: "password",
                                placeholder: "At least 12 characters",
                                value: password.read().to_string(),
                                oninput: move |value| {
                                    password.write().replace(value);
                                    error.set(None);
                                },
                            }
                            Field {
                                label: "Confirm password",
                                input_type: "password",
                                placeholder: "Enter the password again",
                                value: password_confirmation.read().to_string(),
                                oninput: move |value| {
                                    password_confirmation.write().replace(value);
                                    error.set(None);
                                },
                            }

                            if let Some(message) = error.read().as_deref() {
                                ErrorMessage { message: message.to_owned() }
                            }

                            button {
                                class: "button primary submit-button",
                                onclick: move |_| {
                                    let result = {
                                        let pending_wallet = pending_wallet.read();
                                        let entered_phrase = entered_phrase.read();
                                        let password = password.read();
                                        let password_confirmation = password_confirmation.read();

                                        match pending_wallet.as_ref() {
                                            Ok(pending_wallet) => {
                                                save_new_wallet_and_unlock(
                                                    pending_wallet,
                                                    entered_phrase.as_str(),
                                                    password.as_str(),
                                                    password_confirmation.as_str(),
                                                )
                                            }
                                            Err(generation_error) => Err(generation_error.clone()),
                                        }
                                    };
                                    match result {
                                        Ok(wallet) => {
                                            entered_phrase.write().replace(String::new());
                                            password.write().replace(String::new());
                                            password_confirmation.write().replace(String::new());
                                            context.unlocked_wallet.set(Some(wallet));
                                            context.route.set(Route::Wallet);
                                        }
                                        Err(message) => error.set(Some(message)),
                                    }
                                },
                                "Create wallet"
                            }
                        }
                    }
                }
            } else if let Some(message) = generation_error {
                ErrorMessage { message }
            }

        }
    }
}
