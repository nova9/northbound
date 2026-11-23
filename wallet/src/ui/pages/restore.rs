use dioxus::prelude::*;

use crate::config::wallet_directory;
use crate::storage::catalog;
use crate::ui::components::{ErrorMessage, Field};
use crate::ui::routes::Route;
use crate::ui::sensitive_text::SensitiveText;
use crate::ui::state::AppContext;
use crate::wallet::save_restored_wallet_and_unlock;

#[component]
pub(crate) fn Restore() -> Element {
    let mut context = use_context::<AppContext>();
    let navigator = use_navigator();
    let mut wallet_name = use_signal(String::new);
    let mut recovery_phrase = use_signal(SensitiveText::default);
    let mut password = use_signal(SensitiveText::default);
    let mut password_confirmation = use_signal(SensitiveText::default);
    let mut error = use_signal(|| None::<String>);

    rsx! {
        section { class: "screen narrow",
            h1 { "Restore your wallet" }
            p { class: "lede",
                "Enter your 12 recovery words and choose a new password for this device."
            }

            div { class: "form-stack",
                Field {
                    label: "Wallet name", input_type: "text", placeholder: "For example, Savings",
                    value: wallet_name(), oninput: move |value| { wallet_name.set(value); error.set(None); },
                }
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
                    onclick: move |_| {
                        let result = (|| {
                            let directory = wallet_directory()?;
                            let wallet_file =
                                catalog::find_new_wallet_path(&directory, &wallet_name())?;
                            let phrase = recovery_phrase.read();
                            let wallet_password = password.read();
                            let confirmation = password_confirmation.read();
                            let wallet = save_restored_wallet_and_unlock(
                                &wallet_file.path,
                                phrase.as_str(),
                                wallet_password.as_str(),
                                confirmation.as_str(),
                            )?;

                            Ok::<_, String>((wallet_file, wallet))
                        })();

                        match result {
                            Ok((wallet_file, wallet)) => {
                                recovery_phrase.write().replace(String::new());
                                password.write().replace(String::new());
                                password_confirmation.write().replace(String::new());
                                context.selected_wallet.set(Some(wallet_file));
                                context.unlocked_wallet.set(Some(wallet));
                                navigator.replace(Route::Home);
                            }
                            Err(message) => error.set(Some(message)),
                        }
                    },
                    "Restore wallet"
                }
            }

            if let Some(message) = error.read().as_deref() {
                ErrorMessage { message: message.to_owned() }
            }
        }
    }
}
