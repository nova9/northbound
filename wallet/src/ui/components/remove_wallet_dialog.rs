use dioxus::prelude::*;

use crate::config::wallet_directory;
use crate::storage::catalog::{WalletFile, remove_wallet_file};
use crate::ui::components::{ErrorMessage, Field, ModalDialog};
use crate::ui::state::AppContext;

#[component]
pub(crate) fn RemoveWalletDialog(
    wallet: WalletFile,
    on_cancel: EventHandler<()>,
    on_removed: EventHandler<()>,
) -> Element {
    let mut context = use_context::<AppContext>();
    let mut confirmed_name = use_signal(String::new);
    let mut backup_confirmed = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let can_remove = backup_confirmed() && confirmed_name() == wallet.name;

    rsx! {
        ModalDialog {
            title: "Remove {wallet.name}?",
            class: "wallet-removal",
            on_cancel,
            p { "This permanently deletes this wallet’s encrypted file from this device. It does not move any coins. You will need your recovery words to restore access." }
            label { class: "backup-confirmation",
                input {
                    r#type: "checkbox", checked: backup_confirmed(),
                    autofocus: true,
                    oninput: move |event| backup_confirmed.set(event.checked()),
                }
                span { "I have backed up this wallet’s twelve recovery words." }
            }
            Field {
                label: "Type the wallet name to confirm",
                input_type: "text", placeholder: "Exact wallet name", autocomplete: "off",
                value: confirmed_name(), oninput: move |value| confirmed_name.set(value),
            }
            if let Some(message) = error.read().as_deref() {
                ErrorMessage { message: message.to_owned() }
            }
            div { class: "actions-row",
                button {
                    class: "button secondary", r#type: "button",
                    onclick: move |_| on_cancel.call(()), "Keep wallet"
                }
                button {
                    class: "button danger", r#type: "button", disabled: !can_remove,
                    onclick: move |_| {
                        let result = wallet_directory().and_then(|folder| {
                            remove_wallet_file(&folder, &wallet, &confirmed_name(), backup_confirmed())
                        });
                        match result {
                            Ok(()) => {
                                let removed_selected = context.selected_wallet.read().as_ref() == Some(&wallet);
                                if removed_selected {
                                    context.unlocked_wallet.set(None);
                                    context.selected_wallet.set(None);
                                }
                                context.push_header_message(format!("Removed {} from this device.", wallet.name));
                                on_removed.call(());
                            }
                            Err(message) => error.set(Some(message)),
                        }
                    },
                    "Remove wallet file"
                }
            }
        }
    }
}
