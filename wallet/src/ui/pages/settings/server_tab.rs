use dioxus::prelude::*;

use crate::config::DEFAULT_API_BASE_URL;
use crate::server::ServerClient;
use crate::storage::settings;
use crate::ui::components::{AlertDialog, AlertTone, Field};

#[component]
pub(super) fn ServerTab() -> Element {
    let initial_custom_server = settings::custom_api_base_url();
    let mut custom_server_url = use_signal(|| {
        initial_custom_server
            .as_ref()
            .ok()
            .and_then(Clone::clone)
            .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_owned())
    });
    let mut server_is_custom =
        use_signal(|| initial_custom_server.as_ref().is_ok_and(Option::is_some));
    let mut error = use_signal(|| initial_custom_server.err());
    let mut saved = use_signal(|| false);
    let mut pending_check = use_signal(|| None::<dioxus::dioxus_core::Task>);
    let mut allow_override = use_signal(|| false);

    let mut clear_check = move || {
        if let Some(task) = pending_check.write().take() {
            task.cancel();
        }
        allow_override.set(false);
    };

    use_drop(move || {
        if let Some(task) = pending_check.write().take() {
            task.cancel();
        }
    });

    rsx! {
        div { class: "panel-scroll settings-body",
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
                        clear_check();
                        custom_server_url.set(value);
                        error.set(None);
                        saved.set(false);
                    },
                }
                p { class: "server-default-note",
                    "Default: " code { "{DEFAULT_API_BASE_URL}" }
                }
                div { class: "actions-row",
                    button {
                        class: if allow_override() { "button danger" } else { "button primary" },
                        r#type: "button",
                        disabled: pending_check().is_some(),
                        onclick: move |_| {
                            let url = match settings::validate_and_normalize_api_base_url(&custom_server_url()) {
                                Ok(url) => url,
                                Err(message) => {
                                    error.set(Some(message));
                                    return;
                                }
                            };
                            let skip_check = allow_override();
                            saved.set(false);
                            let task = spawn(async move {
                                if !skip_check {
                                    let check_result = match ServerClient::new(&url) {
                                        Ok(server) => server.check_compatibility().await,
                                        Err(message) => Err(message),
                                    };
                                    if let Err(warning) = check_result {
                                        error.set(Some(warning));
                                        allow_override.set(true);
                                        pending_check.set(None);
                                        return;
                                    }
                                }
                                match settings::save_custom_api_base_url(&url) {
                                    Ok(url) => {
                                        custom_server_url.set(url);
                                        server_is_custom.set(true);
                                        error.set(None);
                                        saved.set(true);
                                    }
                                    Err(message) => {
                                        error.set(Some(message));
                                        saved.set(false);
                                    }
                                }
                                allow_override.set(false);
                                pending_check.set(None);
                            });
                            pending_check.set(Some(task));
                        },
                        if pending_check().is_some() { "Checking…" }
                        else if allow_override() { "Save anyway" }
                        else { "Save server" }
                    }
                    button {
                        class: "button secondary",
                        r#type: "button",
                        disabled: !server_is_custom(),
                        onclick: move |_| {
                            clear_check();
                            match settings::reset_api_base_url_to_default() {
                                Ok(()) => {
                                    custom_server_url.set(DEFAULT_API_BASE_URL.to_owned());
                                    server_is_custom.set(false);
                                    error.set(None);
                                    saved.set(false);
                                }
                                Err(message) => error.set(Some(message)),
                            }
                        },
                        "Reset to default"
                    }
                }
                if let Some(message) = error.read().as_deref() {
                    p { class: "error-message", role: "alert", "{message}" }
                }
            }
        }

        if saved() {
            AlertDialog {
                title: "Wallet server saved",
                message: "New requests will use this server URL.",
                dismiss_label: "Done",
                tone: AlertTone::Success,
                on_dismiss: move |_| saved.set(false),
            }
        }
    }
}
