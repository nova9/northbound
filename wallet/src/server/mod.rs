mod addresses;
pub(crate) mod client;
pub(crate) mod discovery;
mod history;

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use client::ServerClient;
pub(crate) use discovery::{ReceivingAddress, WalletAddressScanner};
