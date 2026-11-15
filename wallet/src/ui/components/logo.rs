use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dioxus::prelude::*;

#[component]
pub(crate) fn Logo(size: String) -> Element {
    // Embed the logo so installed desktop builds do not need an asset-file lookup.
    let logo = STANDARD.encode(include_bytes!("../../../assets/logo.svg"));

    rsx! {
        img {
            src: "data:image/svg+xml;base64,{logo}",
            width: size.clone(),
            height: size,
            alt: "Northbound",
        }
    }
}
