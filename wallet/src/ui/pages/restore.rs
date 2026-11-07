use dioxus::prelude::*;

use crate::local_wallet::prepare_restored_wallet;
use crate::ui::components::{ErrorMessage, Field};
use crate::ui::routes::Route;
use crate::ui::sensitive_text::SensitiveText;
use crate::ui::state::AppContext;

#[component]
pub(crate) fn Restore() -> Element {
    let mut context = use_context::<AppContext>();
    let navigator = use_navigator();
    let mut recovery_phrase = use_signal(SensitiveText::default);
    let mut password = use_signal(SensitiveText::default);
    let mut password_confirmation = use_signal(SensitiveText::default);
    let mut error = use_signal(|| None::<String>);
    let mut is_scanning = use_signal(|| false);

    rsx! {
        section { class: "screen narrow",
            h1 { "Restore your wallet" }
            p { class: "lede",
                "Enter your 12 recovery words and choose a new password for this device."
            }

            div { class: "form-stack",
                Field {
                    label: "Recovery phrase",
                    input_type: "text",
                    placeholder: "Twelve recovery words",
                    autocomplete: "off",
                    value: recovery_phrase.read().to_string(),
                    oninput: move |value| {
                        recovery_phrase.write().replace(value);
                        error.set(None);
                    },
                }
                Field {
                    label: "New wallet password",
                    input_type: "password",
                    placeholder: "At least 12 characters",
                    value: password.read().to_string(),
                    oninput: move |value| {
                        password.write().replace(value);
                        error.set(None);
                    },
                }
                Field {
                    label: "Confirm new password",
                    input_type: "password",
                    placeholder: "Enter the password again",
                    value: password_confirmation.read().to_string(),
                    oninput: move |value| {
                        password_confirmation.write().replace(value);
                        error.set(None);
                    },
                }
                button {
                    class: "button primary submit-button",
                    r#type: "button",
                    disabled: is_scanning(),
                    onclick: move |_| {
                        if is_scanning() {
                            return;
                        }

                        let phrase = recovery_phrase.read().snapshot();
                        let wallet_password = password.read().snapshot();
                        let confirmation = password_confirmation.read().snapshot();
                        error.set(None);
                        is_scanning.set(true);

                        spawn(async move {
                            let result = async {
                                let mut wallet = prepare_restored_wallet(
                                    &phrase,
                                    &wallet_password,
                                    &confirmation,
                                )?;
                                let indexes = wallet.scan_recovery_address_indexes().await?;
                                wallet.persist_recovered_address_indexes(indexes)?;

                                Ok::<_, String>(wallet)
                            }
                                .await;

                            is_scanning.set(false);

                            match result {
                                Ok(wallet) => {
                                    recovery_phrase.write().replace(String::new());
                                    password.write().replace(String::new());
                                    password_confirmation.write().replace(String::new());
                                    context.unlocked_wallet.set(Some(wallet));
                                    navigator.replace(Route::Home);
                                }
                                Err(message) => error.set(Some(message)),
                            }
                        });
                    },
                    if is_scanning() {
                        "Scanning wallet history…"
                    } else {
                        "Restore wallet"
                    }
                }
            }

            if let Some(message) = error.read().as_deref() {
                ErrorMessage { message: message.to_owned() }
            }
        }
    }
}
