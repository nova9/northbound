use crate::{ChainTip, Error, IndexDatabase, Result};
use truenorth_rpc::NodeClient;

#[derive(Debug, PartialEq, Eq)]
pub enum SyncProgress {
    Applied(ChainTip),
    RolledBack(ChainTip),
    CaughtUp(ChainTip),
    ChainChanged,
}

/// Reconcile one block at a time. No writes occur after an RPC failure.
pub async fn synchronize_next_block(
    node: &NodeClient,
    database: &mut IndexDatabase,
) -> Result<SyncProgress> {
    let node_tip = node.fetch_tip().await?;
    if node_tip.pruned {
        return Err(Error::InvalidData(
            "indexing requires an unpruned node with all historical blocks available".into(),
        ));
    }
    database.record_node_observation(&node_tip)?;
    let saved_tip = database.tip()?;
    if let Some(tip) = &saved_tip {
        let replaced =
            tip.height > node_tip.blocks || node.fetch_block_hash(tip.height).await? != tip.hash;
        if replaced {
            database.rollback_tip(tip)?;
            return Ok(SyncProgress::RolledBack(tip.clone()));
        }
        if tip.height == node_tip.blocks {
            // The two observations must agree before reporting caught up.
            if hex::encode(tip.hash) != node_tip.bestblockhash.to_ascii_lowercase() {
                return Ok(SyncProgress::ChainChanged);
            }
            return Ok(SyncProgress::CaughtUp(tip.clone()));
        }
    }

    let next_height = match saved_tip.as_ref() {
        Some(tip) => tip
            .height
            .checked_add(1)
            .ok_or_else(|| Error::InvalidData("indexed height overflow".into()))?,
        None => 0,
    };
    let hash = node.fetch_block_hash(next_height).await?;
    let block = node.fetch_block(&hash).await?;
    if block.height != next_height {
        return Err(Error::InvalidData(
            "node returned an unexpected block height".into(),
        ));
    }
    let expected_parent = saved_tip.as_ref().map(|tip| tip.hash);
    if block.previous_hash != expected_parent || node.fetch_block_hash(next_height).await? != hash {
        return Ok(SyncProgress::ChainChanged);
    }
    database.apply_block(&block).map_err(|error| {
        Error::InvalidData(format!(
            "cannot apply block {} at height {next_height}: {error}",
            hex::encode(hash)
        ))
    })?;
    Ok(SyncProgress::Applied(ChainTip {
        height: next_height,
        hash,
    }))
}
