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
}

impl ServerConfig {
    pub fn from_environment() -> Result<Self, String> {
        let network = env::var("TRUENORTH_NETWORK").unwrap_or_else(|_| "regtest".to_owned());

        if network != "main" && network != "regtest" {
            return Err("TRUENORTH_NETWORK must be 'main' or 'regtest'".to_owned());
        }

        let default_rpc_url = if network == "main" {
            "http://127.0.0.1:9554"
        } else {
            "http://127.0.0.1:18443"
        };
        let default_cookie_path = if network == "main" {
            ".local/truenorth/.cookie"
        } else {
            ".local/truenorth/regtest/.cookie"
        };

        let bind_address = env::var("TRUENORTH_SERVER_BIND")
            .unwrap_or_else(|_| "127.0.0.1:3000".to_owned())
            .parse::<SocketAddr>()
            .map_err(|_| "TRUENORTH_SERVER_BIND must be a valid IP address and port".to_owned())?;

        let index_poll_seconds = env::var("TRUENORTH_INDEX_POLL_SECONDS")
            .unwrap_or_else(|_| "2".into())
            .parse::<u64>()
            .map_err(|_| "TRUENORTH_INDEX_POLL_SECONDS must be an integer".to_owned())?;
        if !(1..=10).contains(&index_poll_seconds) {
            return Err(
                "TRUENORTH_INDEX_POLL_SECONDS must be between 1 and 10 for server freshness".into(),
            );
        }
        Ok(Self {
            bind_address,
            rpc_url: env::var("TRUENORTH_RPC_URL").unwrap_or_else(|_| default_rpc_url.to_owned()),
            rpc_cookie_path: env::var("TRUENORTH_RPC_COOKIE_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(default_cookie_path)),
            network,
            index_database: env::var_os("TRUENORTH_INDEX_DATABASE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".local/indexer/chain.sqlite")),
            index_poll_seconds,
        })
    }
}
