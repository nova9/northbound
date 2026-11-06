use dioxus::prelude::*;

use crate::config::DEFAULT_API_BASE_URL;
use crate::ui::components::{AlertDialog, AlertTone, Field};
use crate::ui::sensitive_text::SensitiveText;
use crate::ui::state::AppContext;

#[component]
pub(crate) fn SecurityScreen() -> Element {
    let mut context = use_context::<AppContext>();
    let mut active = use_signal(|| "Backup");
    let mut backup_verification_visible = use_signal(|| false);
    let mut recovery_phrase = use_signal(SensitiveText::default);
    let mut backup_error = use_signal(|| None::<String>);
    let mut backup_verified = use_signal(|| false);
    let mut current_password = use_signal(SensitiveText::default);
    let mut new_password = use_signal(SensitiveText::default);
    let mut new_password_confirmation = use_signal(SensitiveText::default);
    let mut password_error = use_signal(|| None::<String>);
    let mut password_changed = use_signal(|| false);
    let initial_custom_server = crate::config::custom_api_base_url();
    let mut custom_server_url = use_signal(|| {
        initial_custom_server
            .as_ref()
            .ok()
            .and_then(Clone::clone)
            .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_owned())
    });
    let mut server_is_custom =
        use_signal(|| initial_custom_server.as_ref().is_ok_and(Option::is_some));
    let mut server_error = use_signal(|| initial_custom_server.err());
    let mut server_saved = use_signal(|| false);

    rsx! {
        section { class: "screen workspace-screen",
            div { class: "page-heading",
                h1 { "Security" }
                p { "Protect access to your wallet and plan for recovery." }
            }
            div { class: "section-tabs", aria_label: "Security sections",
                for tab in ["Backup", "Password", "Server"] {
                    button {
                        class: if active() == tab { "selected" } else { "" },
                        aria_pressed: active() == tab,
                        onclick: move |_| {
                            active.set(tab);
                            backup_verification_visible.set(false);
                            recovery_phrase.write().replace(String::new());
                            backup_error.set(None);
                            backup_verified.set(false);
                            password_error.set(None);
                            password_changed.set(false);
                            server_error.set(None);
                            server_saved.set(false);
                        },
                        "{tab}"
                    }
                }
            }
            div { class: "panel-scroll settings-body",
                match active() {
                    "Backup" => rsx! {
                        h2 { "Your recovery phrase is your backup" }
                        p {
                            "Keep all 12 words, in order, somewhere private and offline. Your device password cannot recover a lost wallet."
                        }
                        div { class: "setting-row",
                            div {
                                strong { "Check your backup" }
                                p { "Practice confirming your recovery words." }
                            }
                            button {
                                class: "button secondary",
                                onclick: move |_| {
                                    backup_verification_visible.set(true);
                                    backup_error.set(None);
                                },
                                "Verify backup"
                            }
                        }

                        if backup_verification_visible() {
                            div { class: "form-stack backup-verification-form",
                                Field {
                                    label: "Recovery phrase",
                                    input_type: "text",
                                    placeholder: "Enter all 12 words in order",
                                    autocomplete: "off",
                                    value: recovery_phrase.read().to_string(),
                                    oninput: move |value| {
                                        recovery_phrase.write().replace(value);
                                        backup_error.set(None);
                                    },
                                }
                                p { class: "backup-privacy-note",
                                    "Checked only on this device. Your recovery phrase is never sent to the server."
                                }
                                div { class: "actions-row",
                                    button {
                                        class: "button primary",
                                        r#type: "button",
                                        onclick: move |_| {
                                            let result = {
                                                let unlocked_wallet = context.unlocked_wallet.read();
                                                let Some(wallet) = unlocked_wallet.as_ref() else {
                                                    backup_error.set(Some(
                                                        "Unlock the wallet again before checking your backup."
                                                            .to_owned(),
                                                    ));
                                                    return;
                                                };
                                                let recovery_phrase = recovery_phrase.read();

                                                wallet.confirms_recovery_phrase(
                                                    recovery_phrase.as_str(),
                                                )
                                            };

                                            recovery_phrase.write().replace(String::new());

                                            match result {
                                                Ok(true) => {
                                                    backup_error.set(None);
                                                    backup_verification_visible.set(false);
                                                    backup_verified.set(true);
                                                }
                                                Ok(false) => backup_error.set(Some(
                                                    "Recovery phrase does not match this wallet. Check the words and their order, then try again."
                                                        .to_owned(),
                                                )),
                                                Err(_) => backup_error.set(Some(
                                                    "Enter all 12 recovery words exactly as you wrote them down, in the same order."
                                                        .to_owned(),
                                                )),
                                            }
                                        },
                                        "Check recovery phrase"
                                    }
                                    button {
                                        class: "button secondary",
                                        r#type: "button",
                                        onclick: move |_| {
                                            recovery_phrase.write().replace(String::new());
                                            backup_error.set(None);
                                            backup_verification_visible.set(false);
                                        },
                                        "Cancel"
                                    }
                                }
                            }
                        }
                    },
                    "Password" => rsx! {
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
                                    password_error.set(None);
                                    password_changed.set(false);
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
                                    password_error.set(None);
                                    password_changed.set(false);
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
                                    password_error.set(None);
                                    password_changed.set(false);
                                },
                            }

                            button {
                                class: "button primary submit-button",
                                r#type: "button",
                                onclick: move |_| {
                                    let result = {
                                        let mut unlocked_wallet = context.unlocked_wallet.write();
                                        let Some(wallet) = unlocked_wallet.as_mut() else {
                                            password_error.set(Some(
                                                "Unlock the wallet again before changing its password."
                                                    .to_owned(),
                                            ));
                                            return;
                                        };
                                        let current_password = current_password.read();
                                        let new_password = new_password.read();
                                        let new_password_confirmation =
                                            new_password_confirmation.read();

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
                                            new_password_confirmation
                                                .write()
                                                .replace(String::new());
                                            password_error.set(None);
                                            password_changed.set(true);
                                        }
                                        Err(error) => {
                                            password_changed.set(false);
                                            password_error
                                                .set(Some(friendly_password_error(&error)));
                                        }
                                    }
                                },
                                "Change password"
                            }
                        }
                    },
                    _ => rsx! {
                        h2 { "Wallet server" }
                        p {
                            "Choose the server used for balances, fees, transaction history, and broadcasting. Recovery words and private keys never leave this device."
                        }
                        div { class: "form-stack server-settings-form",
                            Field {
                                label: "Server URL",
                                input_type: "url",
                                placeholder: DEFAULT_API_BASE_URL,
                                autocomplete: "url",
                                value: custom_server_url(),
                                oninput: move |value| {
                                    custom_server_url.set(value);
                                    server_error.set(None);
                                    server_saved.set(false);
                                },
                            }
                            p { class: "server-default-note",
                                "Default: " code { "{DEFAULT_API_BASE_URL}" }
                            }
                            div { class: "actions-row",
                                button {
                                    class: "button primary",
                                    r#type: "button",
                                    onclick: move |_| {
                                        match crate::config::save_custom_api_base_url(&custom_server_url()) {
                                            Ok(url) => {
                                                custom_server_url.set(url);
                                                server_is_custom.set(true);
                                                server_error.set(None);
                                                server_saved.set(true);
                                            }
                                            Err(error) => {
                                                server_error.set(Some(error));
                                                server_saved.set(false);
                                            }
                                        }
                                    },
                                    "Save server"
                                }
                                button {
                                    class: "button secondary",
                                    r#type: "button",
                                    disabled: !server_is_custom(),
                                    onclick: move |_| {
                                        match crate::config::reset_api_base_url_to_default() {
                                            Ok(()) => {
                                                custom_server_url.set(DEFAULT_API_BASE_URL.to_owned());
                                                server_is_custom.set(false);
                                                server_error.set(None);
                                                server_saved.set(false);
                                            }
                                            Err(error) => server_error.set(Some(error)),
                                        }
                                    },
                                    "Reset to default"
                                }
                            }
                            if server_saved() {
                                p { class: "success-message", role: "status",
                                    "Wallet server saved. New requests will use this URL."
                                }
                            }
                            if let Some(message) = server_error.read().as_deref() {
                                p { class: "error-message", role: "alert", "{message}" }
                            }
                        }
                    },
                }
            }

            if backup_verified() {
                AlertDialog {
                    title: "Backup verified",
                    message: "This recovery phrase matches your wallet. Keep your written copy private and offline.",
                    dismiss_label: "Done",
                    tone: AlertTone::Success,
                    on_dismiss: move |_| backup_verified.set(false),
                }
            }

            if let Some(error) = backup_error.read().as_deref() {
                AlertDialog {
                    title: "Backup not verified",
                    message: error.to_owned(),
                    dismiss_label: "Try again",
                    tone: AlertTone::Danger,
                    on_dismiss: move |_| backup_error.set(None),
                }
            }

            if let Some(error) = password_error.read().as_deref() {
                AlertDialog {
                    title: "Password not changed",
                    message: error.to_owned(),
                    dismiss_label: "Try again",
                    tone: AlertTone::Danger,
                    on_dismiss: move |_| password_error.set(None),
                }
            }

            if password_changed() {
                AlertDialog {
                    title: "Password changed",
                    message: "Your wallet is now protected by the new password. Use it the next time you unlock this wallet.",
                    dismiss_label: "Done",
                    tone: AlertTone::Success,
                    on_dismiss: move |_| password_changed.set(false),
                }
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
