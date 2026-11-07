use super::logo::Logo;
use crate::config::WALLET_NAME;
use crate::config::WALLET_NETWORK;
use crate::ui::routes::Route;
use crate::ui::state::AppContext;
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum ConnectionStatus {
    Checking,
    Connected,
    Disconnected,
}

const MESSAGE_DISPLAY_TIME: std::time::Duration = std::time::Duration::from_secs(4);
const EMPTY_QUEUE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

#[component]
pub(crate) fn Header() -> Element {
    let mut context = use_context::<AppContext>();

    let mut connection_status = use_signal(|| ConnectionStatus::Checking);
    let mut visible_message = use_signal(|| None::<String>);

    let route = use_route::<Route>();
    let onboarding = matches!(route, Route::Welcome | Route::NewWallet | Route::Restore);

    #[cfg(target_os = "macos")]
    let drag = {
        let window = dioxus::desktop::window();
        rsx! {
            div {
                class: "titlebar-drag-region",
                onmousedown: move |_| window.drag(),
            }
        }
    };
    #[cfg(not(target_os = "macos"))]
    let drag = rsx! {};

    let (connection_class, connection_label) = match *connection_status.read() {
        ConnectionStatus::Checking => ("checking", "Connecting"),
        ConnectionStatus::Connected => ("connected", "Connected"),
        ConnectionStatus::Disconnected => ("disconnected", "Offline"),
    };

    use_future(move || async move {
        loop {
            let status = match crate::api::fetch_chain_height().await {
                Ok(_) => ConnectionStatus::Connected,
                Err(_) => ConnectionStatus::Disconnected,
            };

            if status != *connection_status.read() {
                connection_status.set(status);
            }

            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    });

    use_effect(move || {
        let message = match *connection_status.read() {
            ConnectionStatus::Connected => "Connected to TrueNorth.",
            ConnectionStatus::Disconnected => "Connection lost. Northbound will keep trying.",
            ConnectionStatus::Checking => "Checking the TrueNorth connection.",
        };

        context.push_header_message(message);
    });

    use_future(move || async move {
        loop {
            let next_message = context.header_messages.write().pop_front();

            let Some(next_message) = next_message else {
                tokio::time::sleep(EMPTY_QUEUE_CHECK_INTERVAL).await;
                continue;
            };

            visible_message.set(Some(next_message));
            tokio::time::sleep(MESSAGE_DISPLAY_TIME).await;
            visible_message.set(None);
        }
    });

    let current_message = visible_message.read().clone();

    rsx! {
        header { class: "header",
            {drag}
            div { class: "header-inner",
                div { class: "product-identity",
                    Logo { size: "39".to_string() }
                    strong { "{WALLET_NAME}" }
                }
                if onboarding {
                    nav { class: "setup-nav", aria_label: "Wallet setup",
                        for (target , label) in [
                            (Route::Welcome, "Welcome"),
                            (Route::NewWallet, "Create wallet"),
                            (Route::Restore, "Restore wallet"),
                        ]
                        {
                            Link {
                                to: target.clone(),
                                class: if route == target { "text-button active" } else { "text-button" },
                                "{label}"
                            }
                        }
                    }
                }
                div {
                    class: "header-messages",
                    role: "status",
                    aria_live: "polite",
                    aria_atomic: "true",
                    if let Some(message) = current_message {
                        span { class: "header-message", "{message}" }
                    }
                }
                div { class: "header-status",
                    if WALLET_NETWORK.is_test_network() {
                        span { class: "network", "Regtest" }
                    }

                    div {
                        class: "connection-status",
                        title: "Connection to TrueNorth",
                        span {
                            class: "connection-dot {connection_class}",
                            aria_hidden: "true",
                        }
                        span { "{connection_label}" }
                    }

                }
            }
        }
    }
}
