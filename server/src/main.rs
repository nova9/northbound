use std::sync::Arc;
use std::time::Duration;

use indexer::IndexDatabase;
use server::{AppState, build_router, config::ServerConfig};
use tokio::sync::watch;
use truenorth_rpc::NodeClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig::from_environment()?;
    let node = NodeClient::new(&config.rpc_url, config.rpc_cookie_path, &config.network)?;
    let tip = node.fetch_tip().await?;
    if tip.pruned {
        return Err("The indexer requires an unpruned TrueNorth node.".into());
    }
    let genesis = node.fetch_block_hash(0).await?;
    if let Some(parent) = config
        .index_database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut database = IndexDatabase::open(&config.index_database, &config.network, genesis)?;
    let state = Arc::new(AppState::new(
        node.clone(),
        config.index_database,
        config.network,
    ));
    let listener = tokio::net::TcpListener::bind(config.bind_address).await?;
    let (shutdown, mut worker_stop) = watch::channel(false);
    let mut server_stop = worker_stop.clone();
    // Keep SQLite writes on a dedicated thread; API reads use bounded,
    // independent read-only connections on blocking threads.
    let mut worker = tokio::task::spawn_blocking(move || -> indexer::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            tokio::select! {
                result = indexer::run_worker(&node, &mut database, Duration::from_secs(config.index_poll_seconds), false) => result,
                _ = worker_stop.changed() => database.mark_unavailable(),
            }
        })
    });
    let server = axum::serve(listener, build_router(state, config.requests_per_second))
        .with_graceful_shutdown(async move {
            while !*server_stop.borrow() {
                if server_stop.changed().await.is_err() {
                    break;
                }
            }
        });
    println!(
        "Server listening on http://{}; /ready reports index readiness",
        config.bind_address
    );
    let server = std::future::IntoFuture::into_future(server);
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => {
            shutdown.send_replace(true);
            worker.await??;
            result?;
        }
        result = &mut worker => {
            shutdown.send_replace(true);
            server.await?;
            result??;
            return Err("Indexer worker stopped unexpectedly.".into());
        }
        signal = tokio::signal::ctrl_c() => {
            signal?;
            shutdown.send_replace(true);
            server.await?;
            worker.await??;
        }
    }
    Ok(())
}
