use dioxus::prelude::*;

#[component]
pub(crate) fn Logo(size: String) -> Element {
    rsx! {
        img {
            src: asset!("/assets/logo.svg"),
            width: size.clone(),
            height: size,
            alt: "Northbound",
        }
    }
}
