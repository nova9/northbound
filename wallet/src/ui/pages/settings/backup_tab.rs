use dioxus::prelude::*;

use crate::ui::components::{AlertDialog, AlertTone, Field};
use crate::ui::sensitive_text::SensitiveText;
use crate::ui::state::AppContext;

#[component]
pub(super) fn BackupTab() -> Element {
    let context = use_context::<AppContext>();
    let mut verification_visible = use_signal(|| false);
    let mut recovery_phrase = use_signal(SensitiveText::default);
    let mut error = use_signal(|| None::<String>);
    let mut verified = use_signal(|| false);

    rsx! {
        div { class: "panel-scroll settings-body",
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
                        verification_visible.set(true);
                        error.set(None);
                    },
                    "Verify backup"
                }
            }

            if verification_visible() {
                div { class: "form-stack backup-verification-form",
                    Field {
                        label: "Recovery phrase",
                        input_type: "text",
                        placeholder: "Enter all 12 words in order",
                        autocomplete: "off",
                        value: recovery_phrase.read().to_string(),
                        oninput: move |value| {
                            recovery_phrase.write().replace(value);
                            error.set(None);
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
                                        error.set(Some(
                                            "Unlock the wallet again before checking your backup."
                                                .to_owned(),
                                        ));
                                        return;
                                    };
                                    let recovery_phrase = recovery_phrase.read();

                                    wallet.confirms_recovery_phrase(recovery_phrase.as_str())
                                };

                                recovery_phrase.write().replace(String::new());

                                match result {
                                    Ok(true) => {
                                        error.set(None);
                                        verification_visible.set(false);
                                        verified.set(true);
                                    }
                                    Ok(false) => error.set(Some(
                                        "Recovery phrase does not match this wallet. Check the words and their order, then try again."
                                            .to_owned(),
                                    )),
                                    Err(_) => error.set(Some(
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
                                error.set(None);
                                verification_visible.set(false);
                            },
                            "Cancel"
                        }
                    }
                }
            }
        }

        if verified() {
            AlertDialog {
                title: "Backup verified",
                message: "This recovery phrase matches your wallet. Keep your written copy private and offline.",
                dismiss_label: "Done",
                tone: AlertTone::Success,
                on_dismiss: move |_| verified.set(false),
            }
        }

        if let Some(message) = error.read().as_deref() {
            AlertDialog {
                title: "Backup not verified",
                message: message.to_owned(),
                dismiss_label: "Try again",
                tone: AlertTone::Danger,
                on_dismiss: move |_| error.set(None),
            }
        }
    }
}
