use super::logo::Logo;
use crate::config::WALLET_NAME;
use crate::config::WALLET_NETWORK;
use crate::ui::state::{AppContext, Route};
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum ConnectionStatus {
    Checking,
    Connected,
    Disconnected,
}

#[component]
pub(crate) fn Header() -> Element {
    let mut context = use_context::<AppContext>();

    let mut connection_status = use_signal(|| ConnectionStatus::Checking);

    let route = *context.route.read();
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

            connection_status.set(status);

            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    });

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
                            button {
                                class: if route == target { "text-button active" } else { "text-button" },
                                onclick: move |_| context.route.set(target),
                                "{label}"
                            }
                        }
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
