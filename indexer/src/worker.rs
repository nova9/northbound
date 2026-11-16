use std::time::Duration;

use truenorth_rpc::NodeClient;

use crate::{
    Error, IndexDatabase, MempoolCache, MempoolProgress, Result, SyncProgress, refresh_mempool,
    synchronize_next_block,
};

const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

pub async fn run_worker(
    node: &NodeClient,
    database: &mut IndexDatabase,
    poll: Duration,
    once: bool,
) -> Result<()> {
    let mut cache = MempoolCache::new();
    let mut retry_delay = INITIAL_RETRY_DELAY;
    loop {
        let attempt = async {
            let progress = synchronize_next_block(node, database).await?;
            let mempool_progress = if matches!(progress, SyncProgress::CaughtUp(_)) {
                Some(refresh_mempool(node, database, &mut cache).await?)
            } else {
                None
            };
            Ok((progress, mempool_progress))
        }
        .await;

        let (progress, mempool_progress) = match attempt {
            Ok(outcomes) => {
                retry_delay = INITIAL_RETRY_DELAY;
                outcomes
            }
            Err(Error::Rpc(message)) => {
                database.mark_unavailable()?;

                eprintln!(
                    "Node contact failed; index unavailable. Retrying in {} seconds: {message}",
                    retry_delay.as_secs()
                );

                tokio::time::sleep(retry_delay).await;
                retry_delay = retry_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
                continue;
            }
            Err(error) => {
                database.mark_unavailable()?;
                return Err(error);
            }
        };

        if matches!(mempool_progress, Some(MempoolProgress::ChangedDuringRead)) {
            tokio::time::sleep(poll).await;
            continue;
        }

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
