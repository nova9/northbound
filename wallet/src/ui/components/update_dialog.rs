use dioxus::prelude::*;

use crate::storage::settings;
use crate::ui::components::{ErrorMessage, ModalDialog};
use crate::ui::state::AppContext;
use crate::updates::AvailableUpdate;

#[component]
pub(crate) fn UpdateAvailableDialog(
    update: AvailableUpdate,
    on_dismiss: EventHandler<()>,
) -> Element {
    let mut context = use_context::<AppContext>();
    let mut error = use_signal(|| None::<String>);

    let current_version = env!("CARGO_PKG_VERSION");
    let version_label = if update.prerelease {
        format!("{} (prerelease)", update.tag)
    } else {
        update.tag.clone()
    };
    let skip_version = update.version.clone();
    let skip_tag = update.tag.clone();

    rsx! {
        ModalDialog {
            title: "Update available",
            class: "update-dialog",
            on_cancel: on_dismiss,
            p {
                "Northbound {version_label} is available. You are currently running v{current_version}."
            }
            p {
                "Download the new package from GitHub Releases and verify its checksum before installing. Your encrypted wallet files stay on this device."
            }
            if let Some(message) = error.read().as_deref() {
                ErrorMessage { message: message.to_owned() }
            }
            div { class: "update-dialog-actions",
                button {
                    class: "text-button",
                    r#type: "button",
                    onclick: move |_| match settings::save_skipped_update_version(&skip_version) {
                        Ok(()) => {
                            context.push_header_message(format!(
                                "Skipped Northbound {skip_tag} update."
                            ));
                            on_dismiss.call(());
                        }
                        Err(message) => error.set(Some(message)),
                    },
                    "Skip this version"
                }
                button {
                    class: "button secondary",
                    r#type: "button",
                    onclick: move |_| on_dismiss.call(()),
                    "Later"
                }
                a {
                    class: "button primary update-dialog-link",
                    href: "{update.release_url}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    autofocus: true,
                    "View release"
                    svg {
                        width: "14",
                        height: "14",
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "1.8",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        "aria-hidden": "true",
                        path { d: "M14 5h5v5 M10 14L19 5 M19 14v5H5V5h5" }
                    }
                }
            }
        }
    }
}
