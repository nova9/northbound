use dioxus::prelude::{Signal, WritableExt};
use std::collections::VecDeque;

use crate::local_wallet::UnlockedWallet;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Route {
    Welcome,
    NewWallet,
    Unlock,
    Restore,
    Wallet,
    Receive,
    Send,
    History,
    Security,
    Help,
}

#[derive(Clone, Copy)]
pub(crate) struct AppContext {
    pub(crate) route: Signal<Route>,
    pub(crate) unlocked_wallet: Signal<Option<UnlockedWallet>>,
    pub(crate) header_messages: Signal<VecDeque<String>>,
}

impl AppContext {
    pub(crate) fn push_header_message(&mut self, message: impl Into<String>) {
        self.header_messages.write().push_back(message.into());
    }
}
