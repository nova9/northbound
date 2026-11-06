use std::path::PathBuf;

use crate::p2qrh::Network;

#[cfg(all(feature = "regtest", feature = "mainnet"))]
compile_error!("select exactly one wallet network feature: regtest or mainnet");

#[cfg(not(any(feature = "regtest", feature = "mainnet")))]
compile_error!("select a wallet network feature: regtest or mainnet");

pub const WALLET_NAME: &str = "Northbound";

#[cfg(not(debug_assertions))]
pub const APPLICATION_IDENTIFIER: &str = "com.northbound.app";

pub const WALLET_DESCRIPTION: &str = "A self-custodial wallet for TrueNorth";
pub const CURRENCY_TICKER: &str = "NORTH";

#[cfg(debug_assertions)]
const DEVELOPMENT_WALLET_PATH: &str = ".local/wallet.json";

#[cfg(not(debug_assertions))]
const WALLET_FILE_NAME: &str = "wallet.json";

#[cfg(feature = "regtest")]
pub const WALLET_NETWORK: Network = Network::Regtest;

#[cfg(feature = "mainnet")]
pub const WALLET_NETWORK: Network = Network::Mainnet;

pub const API_BASE_URL: &str = match option_env!("NORTHBOUND_API_BASE_URL") {
    Some(url) => url,
    None => "http://127.0.0.1:3000",
};

#[cfg(debug_assertions)]
pub(crate) fn wallet_path() -> Result<PathBuf, String> {
    Ok(PathBuf::from(DEVELOPMENT_WALLET_PATH))
}

#[cfg(not(debug_assertions))]
pub(crate) fn wallet_path() -> Result<PathBuf, String> {
    let application_data_directory = dirs::data_local_dir()
        .ok_or_else(|| "could not locate the application data directory".to_owned())?;

    Ok(application_data_directory
        .join(APPLICATION_IDENTIFIER)
        .join(WALLET_FILE_NAME))
}
