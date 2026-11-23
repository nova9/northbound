mod backup_tab;
mod password_tab;
mod server_tab;

use dioxus::prelude::*;

use backup_tab::BackupTab;
use password_tab::PasswordTab;
use server_tab::ServerTab;

#[component]
pub(crate) fn Settings() -> Element {
    let mut active = use_signal(|| "Backup");

    rsx! {
        section { class: "screen workspace-screen",
            div { class: "page-heading",
                h1 { "Security" }
                p { "Protect access to your wallet and plan for recovery." }
            }
            div { class: "section-tabs", aria_label: "Security sections",
                for tab in ["Backup", "Password", "Server"] {
                    button {
                        class: if active() == tab { "selected" } else { "" },
                        aria_pressed: active() == tab,
                        onclick: move |_| active.set(tab),
                        "{tab}"
                    }
                }
            }
            match active() {
                "Backup" => rsx! { BackupTab {} },
                "Password" => rsx! { PasswordTab {} },
                _ => rsx! { ServerTab {} },
            }
        }
    }
}
