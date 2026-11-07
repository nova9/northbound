use dioxus::prelude::*;
use std::collections::VecDeque;

use crate::config::WALLET_NAME;
use crate::ui::routes::Route;
use crate::ui::state::AppContext;

const STYLES: &str = include_str!("ui.css");

#[component]
pub fn App() -> Element {
    let unlocked_wallet = use_signal(|| None);
    let header_messages = use_signal(VecDeque::new);

    use_context_provider(|| AppContext {
        unlocked_wallet,
        header_messages,
    });

    rsx! {
        document::Title { "{WALLET_NAME}" }
        style { {STYLES} }
        Router::<Route> {}
    }
}
