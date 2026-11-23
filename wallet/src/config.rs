use std::path::PathBuf;

#[cfg(all(feature = "regtest", feature = "mainnet"))]
compile_error!("select exactly one wallet network feature: regtest or mainnet");

#[cfg(not(any(feature = "regtest", feature = "mainnet")))]
compile_error!("select a wallet network feature: regtest or mainnet");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    #[cfg_attr(feature = "regtest", allow(dead_code))]
    Mainnet,
    #[cfg_attr(feature = "mainnet", allow(dead_code))]
    Regtest,
}

impl Network {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Regtest => "regtest",
        }
    }

    pub const fn server_name(self) -> &'static str {
        match self {
            Self::Mainnet => "main",
            Self::Regtest => "regtest",
        }
    }

    pub const fn is_test_network(self) -> bool {
        !matches!(self, Self::Mainnet)
    }

    pub const fn address_prefix(self) -> &'static str {
        match self {
            Self::Mainnet => "north",
            Self::Regtest => "rnorth",
        }
    }
}

pub const WALLET_NAME: &str = "Northbound";

#[cfg(not(debug_assertions))]
pub const APPLICATION_IDENTIFIER: &str = "io.github.nova9.northbound";

pub const WALLET_DESCRIPTION: &str = "A self-custodial wallet for TrueNorth";
pub const CURRENCY_TICKER: &str = "NORTH";
pub const MAX_ADDRESSES_PER_REQUEST: usize = 100;

#[cfg(feature = "regtest")]
pub const WALLET_NETWORK: Network = Network::Regtest;

#[cfg(feature = "mainnet")]
pub const WALLET_NETWORK: Network = Network::Mainnet;

pub const DEFAULT_API_BASE_URL: &str = match option_env!("NORTHBOUND_API_BASE_URL") {
    Some(url) => url,
    None if cfg!(feature = "regtest") => "http://127.0.0.1:3000",
    None => "https://api.northboundwallet.org",
};

#[cfg(debug_assertions)]
pub(crate) fn settings_path() -> Result<PathBuf, String> {
    Ok(PathBuf::from(".local").join("settings.json"))
}

#[cfg(not(debug_assertions))]
pub(crate) fn settings_path() -> Result<PathBuf, String> {
    let application_data_directory = dirs::data_local_dir()
        .ok_or_else(|| "could not locate the application data directory".to_owned())?;

    Ok(application_data_directory
        .join(APPLICATION_IDENTIFIER)
        .join("settings.json"))
}

#[cfg(debug_assertions)]
pub(crate) fn wallet_directory() -> Result<PathBuf, String> {
    Ok(PathBuf::from(".local/wallets").join(WALLET_NETWORK.name()))
}

#[cfg(not(debug_assertions))]
pub(crate) fn wallet_directory() -> Result<PathBuf, String> {
    let application_data_directory = dirs::data_local_dir()
        .ok_or_else(|| "could not locate the application data directory".to_owned())?;

    Ok(application_data_directory
        .join(APPLICATION_IDENTIFIER)
        .join("wallets")
        .join(WALLET_NETWORK.name()))
}
