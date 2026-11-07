use dioxus::prelude::*;

const SOURCE_CODE_URL: &str = "https://github.com/nova9/northbound";

#[component]
pub(crate) fn Help() -> Element {
    rsx! {
        section { class: "screen workspace-screen help-screen",
            div { class: "page-heading",
                h1 { "Help" }
                p { "Northbound is open source and built for a community that values choice." }
            }

            div { class: "panel-scroll help-content",
                section { class: "help-section",
                    h2 { "Review the source" }
                    p {
                        "All of Northbound's source code is available for anyone to review. You can inspect how the wallet works, report an issue, or contribute an improvement on GitHub."
                    }
                    a {
                        class: "button secondary help-link",
                        href: SOURCE_CODE_URL,
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "View Northbound on GitHub"
                        svg {
                            width: "15",
                            height: "15",
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

                section { class: "help-section",
                    h2 { "Use your own server" }
                    p {
                        "You are welcome to host your own Northbound server and connect this wallet to it. This keeps the service endpoint under your control while your recovery phrase and private keys always remain on this device."
                    }
                    p { class: "help-note",
                        "To change servers, open Security, choose Server, and enter your server URL."
                    }
                }
            }
        }
    }
}
