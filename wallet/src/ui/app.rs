use dioxus::prelude::*;

use crate::config::WALLET_NAME;
use crate::local_wallet;
use crate::ui::components::{Header, Sidebar};
use crate::ui::screens::{
    HelpScreen, HistoryScreen, NewWalletScreen, ReceiveScreen, RestoreScreen, SecurityScreen,
    SendScreen, UnlockScreen, WalletScreen, WelcomeScreen,
};
use crate::ui::state::{AppContext, Route};

const STYLES: &str = include_str!("ui.css");

#[component]
pub fn App() -> Element {
    let initial_route = if local_wallet::exists() {
        Route::Unlock
    } else {
        Route::Welcome
    };

    let route = use_signal(|| initial_route);
    let unlocked_wallet = use_signal(|| None);

    use_context_provider(|| AppContext {
        route,
        unlocked_wallet,
    });

    let current_route = *route.read();
    let wallet_is_open = !matches!(
        current_route,
        Route::Welcome | Route::NewWallet | Route::Restore | Route::Unlock
    );
    let app_shell_class = if cfg!(target_os = "macos") {
        "app-shell macos-titlebar"
    } else {
        "app-shell"
    };

    rsx! {
        document::Title { "{WALLET_NAME}" }
        style { {STYLES} }

        div { class: app_shell_class,
            Header {}
            div { class: if wallet_is_open { "app-body with-sidebar" } else { "app-body onboarding" },
                if wallet_is_open {
                    Sidebar {}
                }
                main { class: "content",
                    match current_route {
                        Route::Welcome => rsx! {
                            WelcomeScreen {}
                        },
                        Route::NewWallet => rsx! {
                            NewWalletScreen {}
                        },
                        Route::Unlock => rsx! {
                            UnlockScreen {}
                        },
                        Route::Restore => rsx! {
                            RestoreScreen {}
                        },
                        Route::Wallet => rsx! {
                            WalletScreen {}
                        },
                        Route::Receive => rsx! {
                            ReceiveScreen {}
                        },
                        Route::Send => rsx! {
                            SendScreen {}
                        },
                        Route::History => rsx! {
                            HistoryScreen {}
                        },
                        Route::Security => rsx! {
                            SecurityScreen {}
                        },
                        Route::Help => rsx! {
                            HelpScreen {}
                        },
                    }
                }
            }
        }
    }
}
