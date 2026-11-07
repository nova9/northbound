use dioxus::prelude::*;

const SOURCE_CODE_URL: &str = "https://github.com/nova9/northbound";

#[component]
pub(crate) fn Faq() -> Element {
    rsx! {
        section { class: "screen workspace-screen faq-screen",
            div { class: "page-heading",
                h1 { "Frequently asked questions" }
                p { "Clear answers about self-custody, servers, recovery, and the current preview." }
            }

            div { class: "panel-scroll faq-content",
                details { class: "help-topic",
                    summary { "What is Northbound?" }
                    p {
                        "Northbound is a lightweight, self-custodial desktop wallet for TrueNorth. It keeps your wallet secrets and transaction signing on this device while using a separate server for public blockchain data."
                    }
                }

                details { class: "help-topic",
                    summary { "Who controls my funds?" }
                    p {
                        "You do. Your recovery words and private keys remain on this device. Northbound cannot recover them for you, so keep an offline backup of your twelve recovery words in a safe place."
                    }
                }

                details { class: "help-topic",
                    summary { "What information does the server receive?" }
                    p {
                        "The server receives public addresses, requests for blockchain data, and signed transactions that are ready to broadcast. It never needs your wallet password, recovery words, seed, or private keys."
                    }
                }

                details { class: "help-topic",
                    summary { "Do I need to run a TrueNorth full node?" }
                    p {
                        "No. Northbound is designed for people who do not want to run a full node. You can use a compatible community server or operate your own Northbound server backed by truenorthd."
                    }
                }

                details { class: "help-topic",
                    summary { "Can a dishonest server steal my funds?" }
                    p {
                        "The server cannot sign a transaction or change the recipient, amount, or fee after you review it. A dishonest server can still hide or delay activity, suggest a poor fee, or refuse to broadcast. Future work will add stronger verification using multiple servers and Electrum-style proofs."
                    }
                }

                details { class: "help-topic",
                    summary { "How do I restore my wallet?" }
                    p {
                        "Choose Restore wallet and enter your twelve recovery words in their original order. Recovery does not depend on the original Northbound server because addresses and keys are derived locally."
                    }
                }

                details { class: "help-topic",
                    summary { "Is Northbound ready for significant funds?" }
                    p {
                        "No. Northbound is an unaudited preview for source review and development. Although the complete flow has been tested on TrueNorth mainnet, there is not yet a public server or downloadable release."
                    }
                }

                details { class: "help-topic",
                    summary { "How can I help?" }
                    p {
                        "Review the wallet security and P2QRH implementation, test on Windows or Linux, improve lightweight verification, or help provide a project domain and reliable public server hosting."
                    }
                    a {
                        class: "text-button faq-source-link",
                        href: SOURCE_CODE_URL,
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "View the source and contribute"
                    }
                }
            }
        }
    }
}
