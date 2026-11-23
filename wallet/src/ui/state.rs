use dioxus::prelude::{Signal, WritableExt};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::storage::catalog::WalletFile;
use crate::wallet::UnlockedWallet;

#[derive(Clone, Copy)]
pub(crate) struct AppContext {
    pub(crate) unlocked_wallet: Signal<Option<UnlockedWallet>>,
    pub(crate) selected_wallet: Signal<Option<WalletFile>>,
    pub(crate) header_messages: Signal<VecDeque<String>>,
    pub(crate) last_seen_receiving_index: Signal<Option<(PathBuf, u32)>>,
}

impl AppContext {
    pub(crate) fn push_header_message(&mut self, message: impl Into<String>) {
        self.header_messages.write().push_back(message.into());
    }

    pub(crate) fn note_active_receiving_index(&mut self, wallet_path: &Path, active_index: u32) {
        let advanced = {
            let mut last_seen = self.last_seen_receiving_index.write();
            let advanced = match last_seen.as_ref() {
                Some((path, previous_index)) if path == wallet_path => {
                    active_index > *previous_index
                }
                _ => false,
            };
            *last_seen = Some((wallet_path.to_path_buf(), active_index));
            advanced
        };

        if advanced {
            self.push_header_message(format!(
                "New receive address ready: Address {active_index}."
            ));
        }
    }
}
