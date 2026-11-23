use dioxus::prelude::*;
use std::time::{Duration, Instant};

use crate::ui::components::{Header, Sidebar, UpdateAvailableDialog};
use crate::ui::routes::Route;
use crate::ui::state::AppContext;
use crate::updates::{AvailableUpdate, check_for_update};

const AUTO_LOCK_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const AUTO_LOCK_CHECK_INTERVAL: Duration = Duration::from_secs(1);

#[component]
pub(crate) fn AppLayout() -> Element {
    let mut context = use_context::<AppContext>();
    let navigator = use_navigator();
    let mut last_activity = use_signal(Instant::now);
    let mut available_update = use_signal(|| None::<AvailableUpdate>);
    let wallet_is_unlocked = context.unlocked_wallet.read().is_some();
    let wallet_view_key = context
        .selected_wallet
        .read()
        .as_ref()
        .map(|wallet| wallet.path.to_string_lossy().into_owned())
        .unwrap_or_default();

    use_future(move || async move {
        if let Some(update) = check_for_update().await {
            available_update.set(Some(update));
        }
    });

    use_future(move || async move {
        loop {
            tokio::time::sleep(AUTO_LOCK_CHECK_INTERVAL).await;

            if context.unlocked_wallet.read().is_none() {
                continue;
            }

            if !inactivity_timeout_reached(*last_activity.read(), Instant::now()) {
                continue;
            }

            context.unlocked_wallet.set(None);
            context.push_header_message("Wallet locked after 15 minutes of inactivity.");
            navigator.replace(Route::Unlock);
        }
    });

    let app_shell_class = if cfg!(target_os = "macos") {
        "app-shell macos-titlebar"
    } else {
        "app-shell"
    };

    let pending_update = available_update.read().clone();

    rsx! {
        div {
            class: app_shell_class,
            onclick: move |_| last_activity.set(Instant::now()),
            onkeydown: move |_| last_activity.set(Instant::now()),
            Header {}
            div { class: if wallet_is_unlocked { "app-body with-sidebar" } else { "app-body onboarding" },
                if wallet_is_unlocked {
                    Sidebar {}
                }
                main { class: "content",
                    // Dioxus applies keys to list children. This single-item list remounts
                    // the pages when the wallet changes, cancelling their old requests.
                    for wallet_key in [wallet_view_key] {
                        Outlet::<Route> { key: "{wallet_key}" }
                    }
                }
            }
            if let Some(update) = pending_update {
                UpdateAvailableDialog {
                    update,
                    on_dismiss: move |_| available_update.set(None),
                }
            }
        }
    }
}

fn inactivity_timeout_reached(last_activity: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_activity) >= AUTO_LOCK_TIMEOUT
}

#[cfg(test)]
mod tests {
    use super::{AUTO_LOCK_TIMEOUT, inactivity_timeout_reached};
    use std::time::{Duration, Instant};

    #[test]
    fn wallet_locks_after_the_full_inactivity_period() {
        let last_activity = Instant::now();

        assert!(!inactivity_timeout_reached(
            last_activity,
            last_activity + AUTO_LOCK_TIMEOUT - Duration::from_secs(1),
        ));
        assert!(inactivity_timeout_reached(
            last_activity,
            last_activity + AUTO_LOCK_TIMEOUT,
        ));
    }
}
