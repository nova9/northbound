use dioxus::prelude::*;

use crate::ui::components::{AlertDialog, AlertTone, Field};
use crate::ui::sensitive_text::SensitiveText;
use crate::ui::state::AppContext;

#[component]
pub(super) fn PasswordTab() -> Element {
    let mut context = use_context::<AppContext>();
    let mut current_password = use_signal(SensitiveText::default);
    let mut new_password = use_signal(SensitiveText::default);
    let mut new_password_confirmation = use_signal(SensitiveText::default);
    let mut error = use_signal(|| None::<String>);
    let mut changed = use_signal(|| false);

    rsx! {
        div { class: "panel-scroll settings-body",
            h2 { "Device access" }
            p {
                "Change the password used to unlock the encrypted wallet on this device. Your recovery phrase does not change."
            }
            div { class: "form-stack change-password-form",
                Field {
                    label: "Current password",
                    input_type: "password",
                    placeholder: "Enter your current password",
                    autocomplete: "current-password",
                    value: current_password.read().to_string(),
                    oninput: move |value| {
                        current_password.write().replace(value);
                        error.set(None);
                        changed.set(false);
                    },
                }
                Field {
                    label: "New password",
                    input_type: "password",
                    placeholder: "At least 12 characters",
                    autocomplete: "new-password",
                    value: new_password.read().to_string(),
                    oninput: move |value| {
                        new_password.write().replace(value);
                        error.set(None);
                        changed.set(false);
                    },
                }
                Field {
                    label: "Confirm new password",
                    input_type: "password",
                    placeholder: "Enter the new password again",
                    autocomplete: "new-password",
                    value: new_password_confirmation.read().to_string(),
                    oninput: move |value| {
                        new_password_confirmation.write().replace(value);
                        error.set(None);
                        changed.set(false);
                    },
                }

                button {
                    class: "button primary submit-button",
                    r#type: "button",
                    onclick: move |_| {
                        let result = {
                            let mut unlocked_wallet = context.unlocked_wallet.write();
                            let Some(wallet) = unlocked_wallet.as_mut() else {
                                error.set(Some(
                                    "Unlock the wallet again before changing its password."
                                        .to_owned(),
                                ));
                                return;
                            };
                            let current_password = current_password.read();
                            let new_password = new_password.read();
                            let new_password_confirmation = new_password_confirmation.read();

                            wallet.change_password(
                                current_password.as_str(),
                                new_password.as_str(),
                                new_password_confirmation.as_str(),
                            )
                        };

                        match result {
                            Ok(()) => {
                                current_password.write().replace(String::new());
                                new_password.write().replace(String::new());
                                new_password_confirmation.write().replace(String::new());
                                error.set(None);
                                changed.set(true);
                            }
                            Err(message) => {
                                changed.set(false);
                                error.set(Some(friendly_password_error(&message)));
                            }
                        }
                    },
                    "Change password"
                }
            }
        }

        if let Some(message) = error.read().as_deref() {
            AlertDialog {
                title: "Password not changed",
                message: message.to_owned(),
                dismiss_label: "Try again",
                tone: AlertTone::Danger,
                on_dismiss: move |_| error.set(None),
            }
        }

        if changed() {
            AlertDialog {
                title: "Password changed",
                message: "Your wallet is now protected by the new password. Use it the next time you unlock this wallet.",
                dismiss_label: "Done",
                tone: AlertTone::Success,
                on_dismiss: move |_| changed.set(false),
            }
        }
    }
}

fn friendly_password_error(error: &str) -> String {
    match error {
        "current wallet password is required" => {
            "Enter your current password, then try again.".to_owned()
        }
        "wallet password must contain at least 12 characters" => {
            "Choose a new password with at least 12 characters.".to_owned()
        }
        "wallet passwords do not match" => {
            "The new passwords do not match. Enter the same password in both fields.".to_owned()
        }
        "new wallet password must be different from the current password" => {
            "Choose a new password that is different from your current password.".to_owned()
        }
        "Incorrect password or damaged wallet file" => {
            "The current password is incorrect. Check it and try again.".to_owned()
        }
        _ => "The password could not be changed. Unlock the wallet again, then try once more."
            .to_owned(),
    }
}
