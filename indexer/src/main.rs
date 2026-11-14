use std::env;
use std::path::PathBuf;
use std::time::Duration;

use indexer::rpc::NodeClient;
use indexer::{Error, IndexDatabase, Result, SyncProgress, synchronize_next_block};

#[tokio::main]
async fn main() {
    let result = tokio::select! {
        result = run() => result,
        signal = tokio::signal::ctrl_c() => signal.map_err(Error::from),
    };
    if let Err(error) = result {
        eprintln!("Indexer stopped: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.iter().any(|argument| argument == "--help") {
        println!(
            "Usage: cargo run -p indexer -- [--once]\n\
            Environment: TRUENORTH_INDEX_DATABASE (required), TRUENORTH_NETWORK,\n\
            TRUENORTH_RPC_URL, TRUENORTH_RPC_COOKIE_PATH, TRUENORTH_INDEX_POLL_SECONDS\n\
            --once catches up and exits; otherwise polls until Ctrl-C."
        );
        return Ok(());
    }
    if arguments.iter().any(|argument| argument != "--once") {
        return Err(Error::InvalidData("unknown argument; use --help".into()));
    }
    let once = arguments.iter().any(|argument| argument == "--once");
    let database_path = env::var_os("TRUENORTH_INDEX_DATABASE").ok_or_else(|| {
        Error::InvalidData("set TRUENORTH_INDEX_DATABASE to a new SQLite file".into())
    })?;
    let network = env::var("TRUENORTH_NETWORK").unwrap_or_else(|_| "regtest".into());
    let (default_url, default_cookie) = if network == "main" {
        ("http://127.0.0.1:9554", ".local/truenorth/.cookie")
    } else {
        ("http://127.0.0.1:18443", ".local/truenorth/regtest/.cookie")
    };
    let url = env::var("TRUENORTH_RPC_URL").unwrap_or_else(|_| default_url.into());
    let cookie = env::var_os("TRUENORTH_RPC_COOKIE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_cookie.into());
    let poll_seconds = env::var("TRUENORTH_INDEX_POLL_SECONDS")
        .unwrap_or_else(|_| "2".into())
        .parse::<u64>()
        .map_err(|_| Error::InvalidData("poll interval must be a positive integer".into()))?;
    if poll_seconds == 0 || poll_seconds > 3600 {
        return Err(Error::InvalidData(
            "poll interval must be between 1 and 3600 seconds".into(),
        ));
    }
    let node = NodeClient::new(&url, cookie, &network)?;
    node.fetch_tip().await?;
    let genesis = node.fetch_block_hash(0).await?;
    let mut database = IndexDatabase::open(&PathBuf::from(database_path), &network, genesis)?;
    let mut rpc_failures = 0_u32;
    loop {
        let progress = match synchronize_next_block(&node, &mut database).await {
            Ok(progress) => {
                rpc_failures = 0;
                progress
            }
            Err(Error::Rpc(message)) if rpc_failures < 5 => {
                rpc_failures += 1;
                eprintln!("Node contact failed; index freshness unknown: {message}");
                tokio::time::sleep(Duration::from_secs(1_u64 << rpc_failures)).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        match progress {
            SyncProgress::Applied(tip) => {
                println!("Indexed {} {}", tip.height, hex::encode(tip.hash))
            }
            SyncProgress::RolledBack(tip) => {
                println!("Removed {} {}", tip.height, hex::encode(tip.hash))
            }
            SyncProgress::CaughtUp(tip) => {
                println!(
                    "Caught up at {} {} (confirmed blocks only)",
                    tip.height,
                    hex::encode(tip.hash)
                );
                if once {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_secs(poll_seconds)).await;
            }
            SyncProgress::ChainChanged => {
                tokio::time::sleep(Duration::from_secs(poll_seconds)).await;
            }
        }
    }
}
