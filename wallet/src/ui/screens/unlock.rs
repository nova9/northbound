use dioxus::prelude::*;

use crate::local_wallet::load_and_unlock_wallet;
use crate::ui::components::{ErrorMessage, Field};
use crate::ui::sensitive_text::SensitiveText;
use crate::ui::state::{AppContext, Route};

#[component]
pub(crate) fn UnlockScreen() -> Element {
    let mut context = use_context::<AppContext>();
    let mut password = use_signal(SensitiveText::default);
    let mut error = use_signal(|| None::<String>);

    rsx! {
        section { class: "screen narrow",
            h1 { "Unlock your wallet" }
            p { class: "lede",
                "Enter your local password. Your recovery phrase and private keys stay on this device."
            }

            div { class: "form-stack",
                Field {
                    label: "Wallet password",
                    input_type: "password",
                    placeholder: "Enter your password",
                    value: password.read().to_string(),
                    oninput: move |value| {
                        password.write().replace(value);
                        error.set(None);
                    },
                }
                button {
                    class: "button primary submit-button",
                    onclick: move |_| {
                        let result = load_and_unlock_wallet(password.read().as_str());

                        match result {
                            Ok(wallet) => {
                                password.write().replace(String::new());
                                context.unlocked_wallet.set(Some(wallet));
                                context.route.set(Route::Wallet);
                                context.push_header_message("Wallet unlocked successfully!");
                            }
                            Err(message) => error.set(Some(message)),
                        }
                    },
                    "Unlock wallet"
                }
            }

            if let Some(message) = error.read().as_deref() {
                ErrorMessage { message: message.to_owned() }
            }
        }
    }
}
