use dioxus::prelude::Signal;

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
}

#[derive(Clone, Copy)]
pub(crate) struct AppContext {
    pub(crate) route: Signal<Route>,
    pub(crate) unlocked_wallet: Signal<Option<UnlockedWallet>>,
}
