use dioxus::prelude::*;
use std::collections::VecDeque;

use crate::config::WALLET_NAME;
use crate::ui::routes::Route;
use crate::ui::state::AppContext;

const STYLES: &str = include_str!("ui.css");

#[component]
pub fn App() -> Element {
    let unlocked_wallet = use_signal(|| None);
    let selected_wallet = use_signal(|| None);
    let header_messages = use_signal(VecDeque::new);
    let last_seen_receiving_index = use_signal(|| None);

    use_context_provider(|| AppContext {
        unlocked_wallet,
        selected_wallet,
        header_messages,
        last_seen_receiving_index,
    });

    rsx! {
        document::Title { "{WALLET_NAME}" }
        style { {STYLES} }
        Router::<Route> {}
    }
}
