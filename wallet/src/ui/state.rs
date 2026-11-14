use dioxus::prelude::{Signal, WritableExt};
use std::collections::VecDeque;

use crate::local_wallet::UnlockedWallet;
use crate::wallet_catalog::WalletFile;

#[derive(Clone, Copy)]
pub(crate) struct AppContext {
    pub(crate) unlocked_wallet: Signal<Option<UnlockedWallet>>,
    pub(crate) selected_wallet: Signal<Option<WalletFile>>,
    pub(crate) header_messages: Signal<VecDeque<String>>,
}

impl AppContext {
    pub(crate) fn push_header_message(&mut self, message: impl Into<String>) {
        self.header_messages.write().push_back(message.into());
    }
}
