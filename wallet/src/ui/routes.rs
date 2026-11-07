use dioxus::prelude::*;

use crate::ui::layout::AppLayout;
use crate::ui::pages::{
    Faq, Help, History, Home, NewWallet, Receive, Restore, Send, Settings, Transaction, Unlock,
    Welcome,
};

#[derive(Clone, Routable, PartialEq)]
#[rustfmt::skip]
pub(crate) enum Route {
    #[layout(AppLayout)]
        #[route("/")]
        Home,

        #[route("/send")]
        Send,

        #[route("/receive")]
        Receive,

        #[route("/history")]
        History,

        #[route("/settings")]
        Settings,

        #[route("/transaction/:txid")]
        Transaction { txid: String },

        #[route("/help")]
        Help,

        #[route("/faq")]
        Faq,

        #[route("/welcome")]
        Welcome,

        #[route("/new-wallet")]
        NewWallet,

        #[route("/restore")]
        Restore,

        #[route("/unlock")]
        Unlock,
}

#[cfg(test)]
mod tests {
    use super::Route;

    #[test]
    fn wallet_routes_have_stable_paths() {
        assert_eq!(Route::Home.to_string(), "/");
        assert_eq!(Route::Send.to_string(), "/send");
        assert_eq!(Route::Receive.to_string(), "/receive");
        assert_eq!(Route::Settings.to_string(), "/settings");
        assert_eq!(Route::Help.to_string(), "/help");
        assert_eq!(Route::Faq.to_string(), "/faq");
        assert_eq!(
            Route::Transaction {
                txid: "transaction-id".to_owned(),
            }
            .to_string(),
            "/transaction/transaction-id"
        );
    }
}
