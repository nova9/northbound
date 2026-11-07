use dioxus::prelude::*;

use crate::ui::components::{Header, Sidebar};
use crate::ui::routes::Route;
use crate::ui::state::AppContext;

#[component]
pub(crate) fn AppLayout() -> Element {
    let context = use_context::<AppContext>();
    let wallet_is_unlocked = context.unlocked_wallet.read().is_some();

    let app_shell_class = if cfg!(target_os = "macos") {
        "app-shell macos-titlebar"
    } else {
        "app-shell"
    };

    rsx! {
        div { class: app_shell_class,
            Header {}
            div { class: if wallet_is_unlocked { "app-body with-sidebar" } else { "app-body onboarding" },
                if wallet_is_unlocked {
                    Sidebar {}
                }
                main { class: "content",
                    Outlet::<Route> {}
                }
            }
        }
    }
}
