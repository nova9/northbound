use crate::config::CURRENCY_TICKER;
use crate::ui::routes::Route;
use crate::ui::state::AppContext;
use dioxus::prelude::*;

#[component]
pub(crate) fn Sidebar() -> Element {
    let mut context = use_context::<AppContext>();
    let current = use_route::<Route>();
    let navigator = use_navigator();
    let items = [
        (
            Route::Home,
            "Overview",
            "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
        ),
        (Route::Send, "Send", "M5 19L19 5 M5 5h14v14"),
        (Route::Receive, "Receive", "M19 5L5 19 M5 5v14h14"),
        (
            Route::History,
            "History",
            "M3 12a9 9 0 1 0 3-7 M3 3v6h6 M12 7v5l3 2",
        ),
        (
            Route::Settings,
            "Security",
            "M12 3l8 3v6c0 5-8 9-8 9s-8-4-8-9V6z M8 12l3 3 5-6",
        ),
        (
            Route::Help,
            "Help",
            "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18 M9.6 9.2a2.6 2.6 0 1 1 3.6 2.4c-.8.4-1.2 1-1.2 1.8 M12 17h.01",
        ),
        (
            Route::Faq,
            "FAQ",
            "M7 7h10 M7 12h7 M7 17h4 M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2",
        ),
    ];
    rsx! {
        aside { class: "sidebar",
            div { class: "wallet-identity",
                strong { "Personal wallet" }
                span { "{CURRENCY_TICKER} · Local keys" }
            }
            nav { class: "sidebar-nav", aria_label: "Wallet navigation",
                for (route , label , path) in items {
                    Link {
                        to: route.clone(),
                        class: if current == route { "sidebar-link active" } else { "sidebar-link" },
                        aria_current: if current == route { "page" } else { "false" },
                        svg {
                            width: "18",
                            height: "18",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "1.6",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            "aria-hidden": "true",
                            path { d: path }
                        }
                        "{label}"
                    }
                }
            }
            div { class: "sidebar-footer",
                span { "Self-custody. On your terms." }
                button {
                    class: "button secondary",
                    r#type: "button",
                    onclick: move |_| {
                        context.unlocked_wallet.set(None);
                        navigator.replace(Route::Unlock);
                    },
                    "Lock wallet"
                }
            }
        }
    }
}
