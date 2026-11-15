use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::p2qrh::Network;

#[cfg(all(feature = "regtest", feature = "mainnet"))]
compile_error!("select exactly one wallet network feature: regtest or mainnet");

#[cfg(not(any(feature = "regtest", feature = "mainnet")))]
compile_error!("select a wallet network feature: regtest or mainnet");

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

#[derive(Default, Deserialize, Serialize)]
struct WalletSettings {
    custom_api_base_url: Option<String>,
}

struct ApiConfiguration {
    custom_url: Option<String>,
    load_error: Option<String>,
}

static API_CONFIGURATION: OnceLock<RwLock<ApiConfiguration>> = OnceLock::new();

pub(crate) fn current_api_base_url() -> Result<String, String> {
    let configuration = api_configuration();
    let configuration = configuration
        .read()
        .map_err(|_| "wallet server settings are unavailable".to_owned())?;

    if let Some(error) = &configuration.load_error {
        return Err(error.clone());
    }

    let url = configuration
        .custom_url
        .as_deref()
        .unwrap_or(DEFAULT_API_BASE_URL);
    validate_and_normalize_api_base_url(url)
}

pub(crate) fn custom_api_base_url() -> Result<Option<String>, String> {
    let configuration = api_configuration();
    let configuration = configuration
        .read()
        .map_err(|_| "wallet server settings are unavailable".to_owned())?;

    if let Some(error) = &configuration.load_error {
        return Err(error.clone());
    }

    Ok(configuration.custom_url.clone())
}

pub(crate) fn save_custom_api_base_url(url: &str) -> Result<String, String> {
    let normalized_url = validate_and_normalize_api_base_url(url)?;
    let settings = WalletSettings {
        custom_api_base_url: Some(normalized_url.clone()),
    };

    write_settings_file(&settings)?;
    replace_api_configuration(Some(normalized_url.clone()))?;

    Ok(normalized_url)
}

pub(crate) fn reset_api_base_url_to_default() -> Result<(), String> {
    write_settings_file(&WalletSettings::default())?;
    replace_api_configuration(None)
}

fn api_configuration() -> &'static RwLock<ApiConfiguration> {
    API_CONFIGURATION.get_or_init(|| {
        let (custom_url, load_error) = match read_settings_file() {
            Ok(settings) => (settings.custom_api_base_url, None),
            Err(error) => (None, Some(error)),
        };

        RwLock::new(ApiConfiguration {
            custom_url,
            load_error,
        })
    })
}

fn replace_api_configuration(custom_url: Option<String>) -> Result<(), String> {
    let configuration = api_configuration();
    let mut configuration = configuration
        .write()
        .map_err(|_| "wallet server settings are unavailable".to_owned())?;
    configuration.custom_url = custom_url;
    configuration.load_error = None;
    Ok(())
}

fn validate_and_normalize_api_base_url(url: &str) -> Result<String, String> {
    let normalized_url = url.trim().trim_end_matches('/');
    if normalized_url.is_empty() {
        return Err("Enter the wallet server URL.".to_owned());
    }

    let parsed_url = reqwest::Url::parse(normalized_url)
        .map_err(|_| "Enter a valid wallet server URL.".to_owned())?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err("Wallet server URLs must start with http:// or https://.".to_owned());
    }
    if parsed_url.host_str().is_none() {
        return Err("The wallet server URL must include a host.".to_owned());
    }
    if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
        return Err("The wallet server URL cannot include credentials.".to_owned());
    }
    let host = parsed_url.host_str().unwrap_or_default();
    let is_loopback = host == "localhost"
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if parsed_url.scheme() == "http" && !is_loopback {
        return Err(
            "Remote wallet servers must use https://. HTTP is only allowed for loopback addresses."
                .to_owned(),
        );
    }
    if parsed_url.query().is_some() || parsed_url.fragment().is_some() {
        return Err("The wallet server URL cannot include a query or fragment.".to_owned());
    }

    Ok(normalized_url.to_owned())
}

fn read_settings_file() -> Result<WalletSettings, String> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(WalletSettings::default());
    }

    let bytes =
        std::fs::read(&path).map_err(|error| format!("could not read wallet settings: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("wallet settings are invalid: {error}"))
}

fn write_settings_file(settings: &WalletSettings) -> Result<(), String> {
    let path = settings_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| "wallet settings path has no parent directory".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create wallet settings directory: {error}"))?;

    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("could not encode wallet settings: {error}"))?;
    std::fs::write(&path, bytes).map_err(|error| format!("could not save wallet settings: {error}"))
}

#[cfg(debug_assertions)]
fn settings_path() -> Result<PathBuf, String> {
    Ok(PathBuf::from(".local").join("settings.json"))
}

#[cfg(not(debug_assertions))]
fn settings_path() -> Result<PathBuf, String> {
    let application_data_directory = dirs::data_local_dir()
        .ok_or_else(|| "could not locate the application data directory".to_owned())?;

    Ok(application_data_directory
        .join(APPLICATION_IDENTIFIER)
        .join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::validate_and_normalize_api_base_url;

    #[test]
    fn server_url_is_trimmed_and_trailing_slashes_are_removed() {
        let url = validate_and_normalize_api_base_url("  https://wallet.example.com///  ")
            .expect("URL should be valid");

        assert_eq!(url, "https://wallet.example.com");
    }

    #[test]
    fn server_url_requires_http_or_https() {
        let error = validate_and_normalize_api_base_url("file:///tmp/server")
            .expect_err("file URLs must be rejected");

        assert_eq!(
            error,
            "Wallet server URLs must start with http:// or https://."
        );
    }

    #[test]
    fn server_url_rejects_queries_and_fragments() {
        assert!(
            validate_and_normalize_api_base_url("https://wallet.example.com?network=main").is_err()
        );
        assert!(validate_and_normalize_api_base_url("https://wallet.example.com#server").is_err());
    }

    #[test]
    fn remote_servers_require_https() {
        for host in [
            "13.55.12.147",
            "wallet.example.com",
            "192.168.1.10",
            "[2001:db8::1]",
            "localhost.example.com",
        ] {
            assert!(validate_and_normalize_api_base_url(&format!("http://{host}")).is_err());
            assert!(validate_and_normalize_api_base_url(&format!("https://{host}")).is_ok());
        }
    }

    #[test]
    fn loopback_servers_allow_http() {
        for host in ["localhost", "127.0.0.1", "127.0.0.2", "[::1]"] {
            assert!(validate_and_normalize_api_base_url(&format!("http://{host}:3000")).is_ok());
        }
    }

    #[test]
    fn server_url_rejects_credentials() {
        assert!(
            validate_and_normalize_api_base_url("https://user:password@wallet.example.com")
                .is_err()
        );
    }

    #[test]
    fn default_server_satisfies_transport_policy() {
        assert!(validate_and_normalize_api_base_url(super::DEFAULT_API_BASE_URL).is_ok());
    }
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
