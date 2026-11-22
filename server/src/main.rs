use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use indexer::IndexDatabase;
use server::{AppState, build_router, config::ServerConfig};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use truenorth_rpc::NodeClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = ServerConfig::from_environment()?;
    let node = connect_node(&config).await?;
    let database = open_index_database(&config, &node).await?;

    let state = Arc::new(AppState::new(
        node.clone(),
        config.index_database.clone(),
        config.network.clone(),
    ));
    let mut router = build_router(state);

    let (shutdown, stop_signal) = watch::channel(false);

    // Solo-mining routes are isolated with their own state and template poller.
    let mining_task = if config.mining_enabled {
        let mining_state = Arc::new(server::mining::MiningState::new(node.clone()));
        let mining_router = server::mining::build_router(mining_state.clone());
        router = router.merge(mining_router);

        Some(tokio::spawn(server::mining::run_template_watcher(
            mining_state,
            stop_signal.clone(),
        )))
    } else {
        None
    };

    let listener = tokio::net::TcpListener::bind(config.bind_address).await?;
    let mut indexer_worker = spawn_indexer_worker(
        node,
        database,
        Duration::from_secs(config.index_poll_seconds),
        stop_signal.clone(),
    );

    println!(
        "Server listening on http://{}; /ready reports index readiness",
        config.bind_address
    );

    let server =
        axum::serve(listener, router).with_graceful_shutdown(wait_for_shutdown_signal(stop_signal));
    let server = std::future::IntoFuture::into_future(server);
    tokio::pin!(server);

    tokio::select! {
        result = &mut server => {
            shutdown.send_replace(true);
            indexer_worker.await??;
            result?;
        }
        result = &mut indexer_worker => {
            shutdown.send_replace(true);
            server.await?;
            result??;
            return Err("Indexer worker stopped unexpectedly.".into());
        }
        signal = tokio::signal::ctrl_c() => {
            signal?;
            shutdown.send_replace(true);
            server.await?;
            indexer_worker.await??;
        }
    }

    if let Some(task) = mining_task {
        task.await?;
    }

    Ok(())
}

async fn connect_node(config: &ServerConfig) -> Result<NodeClient, Box<dyn Error>> {
    let node = NodeClient::new(
        &config.rpc_url,
        config.rpc_cookie_path.clone(),
        &config.network,
    )?;

    let tip = node.fetch_tip().await?;
    if tip.pruned {
        return Err("The indexer requires an unpruned TrueNorth node.".into());
    }

    Ok(node)
}

async fn open_index_database(
    config: &ServerConfig,
    node: &NodeClient,
) -> Result<IndexDatabase, Box<dyn Error>> {
    let genesis = node.fetch_block_hash(0).await?;

    if let Some(parent) = config.index_database.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }

    let database = IndexDatabase::open(&config.index_database, &config.network, genesis)?;
    Ok(database)
}

/// Runs the SQLite indexer writer on a dedicated blocking thread with its own
/// single-threaded runtime, because SQLite connections are synchronous and `!Send`.
fn spawn_indexer_worker(
    node: NodeClient,
    mut database: IndexDatabase,
    poll_interval: Duration,
    mut stop: watch::Receiver<bool>,
) -> JoinHandle<indexer::Result<()>> {
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        runtime.block_on(async {
            tokio::select! {
                result = indexer::run_worker(&node, &mut database, poll_interval) => result,
                _ = stop.changed() => database.mark_unavailable(),
            }
        })
    })
}

async fn wait_for_shutdown_signal(mut stop: watch::Receiver<bool>) {
    while !*stop.borrow() {
        if stop.changed().await.is_err() {
            break;
        }
    }
}
