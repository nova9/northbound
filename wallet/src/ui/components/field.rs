use dioxus::prelude::*;

#[component]
pub(crate) fn Field(
    label: &'static str,
    input_type: &'static str,
    placeholder: &'static str,
    value: String,
    oninput: EventHandler<String>,
    autocomplete: Option<&'static str>,
) -> Element {
    rsx! {
        label { class: "field",
            span { "{label}" }
            input {
                r#type: input_type,
                placeholder,
                value,
                autocomplete: autocomplete.unwrap_or("on"),
                oninput: move |event| oninput.call(event.value()),
            }
        }
    }
}
