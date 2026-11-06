use dioxus::prelude::*;

#[component]
pub(crate) fn ErrorMessage(message: String) -> Element {
    rsx! {
        p { class: "error-message", role: "alert", "{message}" }
    }
}
