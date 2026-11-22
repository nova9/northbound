use std::env;
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
        let network = read_env_or("TRUENORTH_NETWORK", "regtest");
        if network != "main" && network != "regtest" {
            return Err("TRUENORTH_NETWORK must be 'main' or 'regtest'".to_owned());
        }

        let (default_rpc_url, default_cookie_path) = if network == "main" {
            ("http://127.0.0.1:9554", ".local/truenorth/mainnet/.cookie")
        } else {
            ("http://127.0.0.1:18443", ".local/truenorth/regtest/.cookie")
        };

        let bind_address = read_env_or("TRUENORTH_SERVER_BIND", "127.0.0.1:3000")
            .parse::<SocketAddr>()
            .map_err(|_| "TRUENORTH_SERVER_BIND must be a valid IP address and port".to_owned())?;

        let rpc_url = read_env_or("TRUENORTH_RPC_URL", default_rpc_url);
        let rpc_cookie_path = read_env_path("TRUENORTH_RPC_COOKIE_PATH", default_cookie_path);
        let index_database =
            read_env_path("TRUENORTH_INDEX_DATABASE", ".local/indexer/chain.sqlite");

        let index_poll_seconds = read_env_or("TRUENORTH_INDEX_POLL_SECONDS", "2")
            .parse::<u64>()
            .map_err(|_| "TRUENORTH_INDEX_POLL_SECONDS must be an integer".to_owned())?;
        if !(1..=10).contains(&index_poll_seconds) {
            return Err(
                "TRUENORTH_INDEX_POLL_SECONDS must be between 1 and 10 for server freshness"
                    .to_owned(),
            );
        }

        let mining_enabled = read_env_bool("TRUENORTH_MINING_ENABLED", true)?;

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

fn read_env_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn read_env_path(name: &str, default: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn read_env_bool(name: &str, default: bool) -> Result<bool, String> {
    match env::var(name) {
        Err(env::VarError::NotPresent) => Ok(default),
        Ok(value) if value == "true" => Ok(true),
        Ok(value) if value == "false" => Ok(false),
        _ => Err(format!("{name} must be 'true' or 'false'")),
    }
}
