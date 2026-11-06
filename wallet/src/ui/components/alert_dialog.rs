use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum AlertTone {
    Information,
    Success,
    Danger,
}

#[component]
pub(crate) fn AlertDialog(
    title: String,
    message: String,
    dismiss_label: String,
    tone: AlertTone,
    on_dismiss: EventHandler<()>,
) -> Element {
    let tone_class = match tone {
        AlertTone::Information => "alert-dialog information",
        AlertTone::Success => "alert-dialog success",
        AlertTone::Danger => "alert-dialog danger",
    };

    rsx! {
        div { class: "alert-backdrop",
            section {
                class: tone_class,
                role: "alertdialog",
                aria_modal: "true",
                aria_label: title.clone(),
                h2 { "{title}" }
                p { "{message}" }
                div { class: "alert-actions",
                    button {
                        class: "button primary",
                        r#type: "button",
                        autofocus: true,
                        onclick: move |_| on_dismiss.call(()),
                        "{dismiss_label}"
                    }
                }
            }
        }
    }
}
