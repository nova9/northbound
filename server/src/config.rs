use std::env;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;

pub struct ServerConfig {
    pub bind_address: SocketAddr,
    pub rpc_url: String,
    pub rpc_cookie_path: PathBuf,
    pub network: String,
    pub index_database: PathBuf,
    pub index_poll_seconds: u64,
    pub mining_enabled: bool,
}

impl ServerConfig {
    pub fn from_environment() -> Result<Self, String> {
        Self::from_lookup(|name| env::var_os(name))
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Self, String> {
        let network = read_string_or(&lookup, "TRUENORTH_NETWORK", "regtest")?;
        if network != "main" && network != "regtest" {
            return Err("TRUENORTH_NETWORK must be 'main' or 'regtest'".to_owned());
        }

        let (default_rpc_url, default_cookie_path) = if network == "main" {
            ("http://127.0.0.1:9554", ".local/truenorth/mainnet/.cookie")
        } else {
            ("http://127.0.0.1:18443", ".local/truenorth/regtest/.cookie")
        };

        let bind_address = read_string_or(&lookup, "TRUENORTH_SERVER_BIND", "127.0.0.1:3000")?
            .parse::<SocketAddr>()
            .map_err(|_| "TRUENORTH_SERVER_BIND must be a valid IP address and port".to_owned())?;

        let rpc_url = read_string_or(&lookup, "TRUENORTH_RPC_URL", default_rpc_url)?;
        let rpc_cookie_path =
            read_path_or(&lookup, "TRUENORTH_RPC_COOKIE_PATH", default_cookie_path);
        let index_database = read_path_or(
            &lookup,
            "TRUENORTH_INDEX_DATABASE",
            ".local/indexer/chain.sqlite",
        );

        let index_poll_seconds = read_string_or(&lookup, "TRUENORTH_INDEX_POLL_SECONDS", "2")?
            .parse::<u64>()
            .map_err(|_| "TRUENORTH_INDEX_POLL_SECONDS must be an integer".to_owned())?;
        if !(1..=10).contains(&index_poll_seconds) {
            return Err(
                "TRUENORTH_INDEX_POLL_SECONDS must be between 1 and 10 for server freshness"
                    .to_owned(),
            );
        }

        let mining_enabled = read_bool_or(&lookup, "TRUENORTH_MINING_ENABLED", true)?;

        Ok(Self {
            bind_address,
            rpc_url,
            rpc_cookie_path,
            network,
            index_database,
            index_poll_seconds,
            mining_enabled,
        })
    }
}

fn read_string_or(
    lookup: &impl Fn(&str) -> Option<OsString>,
    name: &str,
    default: &str,
) -> Result<String, String> {
    match lookup(name) {
        None => Ok(default.to_owned()),
        Some(value) => value
            .into_string()
            .map_err(|_| format!("{name} must be valid UTF-8")),
    }
}

fn read_path_or(lookup: &impl Fn(&str) -> Option<OsString>, name: &str, default: &str) -> PathBuf {
    lookup(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn read_bool_or(
    lookup: &impl Fn(&str) -> Option<OsString>,
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match lookup(name) {
        None => Ok(default),
        Some(value) if value == "true" => Ok(true),
        Some(value) if value == "false" => Ok(false),
        Some(_) => Err(format!("{name} must be 'true' or 'false'")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::ServerConfig;

    fn config_from(pairs: &[(&str, &str)]) -> Result<ServerConfig, String> {
        let values: BTreeMap<&str, &str> = pairs.iter().copied().collect();
        ServerConfig::from_lookup(|name| values.get(name).map(OsString::from))
    }

    #[test]
    fn regtest_and_mainnet_defaults_select_matching_rpc_endpoints() {
        let regtest = config_from(&[]).unwrap();
        assert_eq!(regtest.network, "regtest");
        assert_eq!(regtest.bind_address.to_string(), "127.0.0.1:3000");
        assert_eq!(regtest.rpc_url, "http://127.0.0.1:18443");
        assert_eq!(
            regtest.rpc_cookie_path,
            PathBuf::from(".local/truenorth/regtest/.cookie")
        );
        assert_eq!(regtest.index_poll_seconds, 2);
        assert!(regtest.mining_enabled);

        let mainnet = config_from(&[("TRUENORTH_NETWORK", "main")]).unwrap();
        assert_eq!(mainnet.network, "main");
        assert_eq!(mainnet.rpc_url, "http://127.0.0.1:9554");
        assert_eq!(
            mainnet.rpc_cookie_path,
            PathBuf::from(".local/truenorth/mainnet/.cookie")
        );
    }

    #[test]
    fn rejects_invalid_network_bind_poll_interval_and_boolean() {
        assert!(config_from(&[("TRUENORTH_NETWORK", "testnet")]).is_err());
        assert!(config_from(&[("TRUENORTH_SERVER_BIND", "not-an-addr")]).is_err());
        assert!(config_from(&[("TRUENORTH_INDEX_POLL_SECONDS", "0")]).is_err());
        assert!(config_from(&[("TRUENORTH_INDEX_POLL_SECONDS", "11")]).is_err());
        assert!(config_from(&[("TRUENORTH_INDEX_POLL_SECONDS", "fast")]).is_err());
        assert!(config_from(&[("TRUENORTH_MINING_ENABLED", "1")]).is_err());

        let valid = config_from(&[
            ("TRUENORTH_INDEX_POLL_SECONDS", "10"),
            ("TRUENORTH_MINING_ENABLED", "false"),
        ])
        .unwrap();
        assert_eq!(valid.index_poll_seconds, 10);
        assert!(!valid.mining_enabled);
    }
}
