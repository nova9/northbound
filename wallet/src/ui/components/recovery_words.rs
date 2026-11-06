use dioxus::prelude::*;

#[component]
pub(crate) fn RecoveryWords(phrase: String) -> Element {
    rsx! {
        ol { class: "recovery-grid",
            for word in phrase.split_whitespace() {
                li { span { "{word}" } }
            }
        }
    }
}
