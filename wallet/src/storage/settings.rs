use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::config::{DEFAULT_API_BASE_URL, settings_path};

#[derive(Default, Deserialize, Serialize)]
struct WalletSettings {
    custom_api_base_url: Option<String>,
}

static WALLET_SETTINGS_CACHE: OnceLock<RwLock<Result<WalletSettings, String>>> = OnceLock::new();

pub(crate) fn current_api_base_url() -> Result<String, String> {
    let custom_url = custom_api_base_url()?;
    validate_and_normalize_api_base_url(custom_url.as_deref().unwrap_or(DEFAULT_API_BASE_URL))
}

pub(crate) fn custom_api_base_url() -> Result<Option<String>, String> {
    let settings = wallet_settings_cache()
        .read()
        .map_err(|_| "wallet server settings are unavailable".to_owned())?;

    match &*settings {
        Ok(settings) => Ok(settings.custom_api_base_url.clone()),
        Err(error) => Err(error.clone()),
    }
}

pub(crate) fn save_custom_api_base_url(url: &str) -> Result<String, String> {
    let normalized_url = validate_and_normalize_api_base_url(url)?;
    save_and_cache_settings(WalletSettings {
        custom_api_base_url: Some(normalized_url.clone()),
    })?;
    Ok(normalized_url)
}

pub(crate) fn reset_api_base_url_to_default() -> Result<(), String> {
    save_and_cache_settings(WalletSettings::default())
}

pub(crate) fn validate_and_normalize_api_base_url(url: &str) -> Result<String, String> {
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

fn wallet_settings_cache() -> &'static RwLock<Result<WalletSettings, String>> {
    WALLET_SETTINGS_CACHE.get_or_init(|| RwLock::new(read_settings_file()))
}

fn save_and_cache_settings(settings: WalletSettings) -> Result<(), String> {
    let mut cached_settings = wallet_settings_cache()
        .write()
        .map_err(|_| "wallet server settings are unavailable".to_owned())?;
    write_settings_file(&settings)?;
    *cached_settings = Ok(settings);
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::validate_and_normalize_api_base_url;
    use crate::config::DEFAULT_API_BASE_URL;

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
        assert!(validate_and_normalize_api_base_url(DEFAULT_API_BASE_URL).is_ok());
    }
}
