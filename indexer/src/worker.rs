use std::time::Duration;

use truenorth_rpc::NodeClient;

use crate::{
    Error, IndexDatabase, MempoolCache, Result, SyncProgress, refresh_mempool,
    synchronize_next_block,
};

pub async fn run_worker(
    node: &NodeClient,
    database: &mut IndexDatabase,
    poll: Duration,
    once: bool,
) -> Result<()> {
    let mut cache = MempoolCache::new();
    let mut failures = 0_u32;
    loop {
        let attempt = async {
            let progress = synchronize_next_block(node, database).await?;
            if matches!(progress, SyncProgress::CaughtUp(_)) {
                refresh_mempool(node, database, &mut cache).await?;
            }
            Ok(progress)
        }
        .await;
        let progress = match attempt {
            Ok(progress) => {
                failures = 0;
                progress
            }
            Err(Error::Rpc(message)) if failures < 5 => {
                database.mark_unavailable()?;
                failures += 1;
                eprintln!("Node contact failed; index unavailable: {message}");
                tokio::time::sleep(Duration::from_secs(1_u64 << failures)).await;
                continue;
            }
            Err(error) => {
                database.mark_unavailable()?;
                return Err(error);
            }
        };
        match progress {
            SyncProgress::Applied(tip) => {
                println!("Indexed {} {}", tip.height, hex::encode(tip.hash))
            }
            SyncProgress::RolledBack(tip) => {
                println!("Removed {} {}", tip.height, hex::encode(tip.hash))
            }
            SyncProgress::CaughtUp(tip) => {
                // A chain change during snapshot capture leaves readiness false.
                let ready = crate::status::read_status(&database.connection)?.ready;
                if ready && once {
                    println!("Caught up at {} {}", tip.height, hex::encode(tip.hash));
                    return Ok(());
                }
                tokio::time::sleep(poll).await;
            }
            SyncProgress::ChainChanged => tokio::time::sleep(poll).await,
        }
    }
}
