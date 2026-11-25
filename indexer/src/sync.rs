use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use truenorth_rpc::{Hash, NodeClient, Transaction, decode_hash};

use crate::{ChainTip, Error, IndexDatabase, Result};

const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

pub type MempoolCache = BTreeMap<Hash, Transaction>;

#[derive(Debug, PartialEq, Eq)]
pub enum SyncProgress {
    Applied(ChainTip),
    RolledBack(ChainTip),
    CaughtUp(ChainTip),
    ChainChanged,
}

pub enum MempoolProgress {
    Ready,
    ChangedDuringRead,
}

// --- Background worker loop ---

/// Continuously synchronizes blocks and mempool snapshots from the node.
///
/// Transient RPC failures mark the index unavailable and retry with exponential
/// backoff (2s up to 30s). Database, identity, and decoding errors stop immediately.
pub async fn run_worker(
    node: &NodeClient,
    database: &mut IndexDatabase,
    poll_interval: Duration,
) -> Result<()> {
    let mut mempool_cache = MempoolCache::new();
    let mut retry_delay = INITIAL_RETRY_DELAY;

    loop {
        let step = run_sync_step(node, database, &mut mempool_cache).await;

        let (block_progress, mempool_progress) = match step {
            Ok(outcome) => {
                retry_delay = INITIAL_RETRY_DELAY;
                outcome
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
            tokio::time::sleep(poll_interval).await;
            continue;
        }

        match block_progress {
            SyncProgress::Applied(tip) => {
                println!("Indexed {} {}", tip.height, hex::encode(tip.hash));
            }
            SyncProgress::RolledBack(tip) => {
                println!("Removed {} {}", tip.height, hex::encode(tip.hash));
            }
            SyncProgress::CaughtUp(_) | SyncProgress::ChainChanged => {
                tokio::time::sleep(poll_interval).await;
            }
        }
    }
}

async fn run_sync_step(
    node: &NodeClient,
    database: &mut IndexDatabase,
    mempool_cache: &mut MempoolCache,
) -> Result<(SyncProgress, Option<MempoolProgress>)> {
    let block_progress = synchronize_next_block(node, database).await?;

    let mempool_progress = if matches!(block_progress, SyncProgress::CaughtUp(_)) {
        Some(refresh_mempool(node, database, mempool_cache).await?)
    } else {
        None
    };

    Ok((block_progress, mempool_progress))
}

// --- Confirmed block synchronization ---

/// Reconciles one block step between the node and the local database.
///
/// If our saved tip no longer matches the node's active chain, rolls back one block.
/// Otherwise, fetches and applies the next block above our saved tip.
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
        let on_stale_branch =
            tip.height > node_tip.blocks || node.fetch_block_hash(tip.height).await? != tip.hash;
        if on_stale_branch {
            database.rollback_tip(tip)?;
            return Ok(SyncProgress::RolledBack(tip.clone()));
        }

        if tip.height == node_tip.blocks {
            // Confirm that both RPC calls agreed on the exact same tip hash.
            if hex::encode(tip.hash) != node_tip.bestblockhash.to_ascii_lowercase() {
                return Ok(SyncProgress::ChainChanged);
            }
            return Ok(SyncProgress::CaughtUp(tip.clone()));
        }
    }

    let next_height = match &saved_tip {
        Some(tip) => tip
            .height
            .checked_add(1)
            .ok_or_else(|| Error::InvalidData("indexed height overflow".into()))?,
        None => 0,
    };

    let block_hash = node.fetch_block_hash(next_height).await?;
    let block = node.fetch_block(&block_hash).await?;
    if block.height != next_height {
        return Err(Error::InvalidData(
            "node returned an unexpected block height".into(),
        ));
    }

    // Re-verify parent and active-chain hash in case a reorg happened while fetching the block.
    let expected_parent = saved_tip.as_ref().map(|tip| tip.hash);
    if block.previous_hash != expected_parent
        || node.fetch_block_hash(next_height).await? != block_hash
    {
        return Ok(SyncProgress::ChainChanged);
    }

    database.apply_block(&block).map_err(|error| {
        Error::InvalidData(format!(
            "cannot apply block {} at height {next_height}: {error}",
            hex::encode(block_hash)
        ))
    })?;

    Ok(SyncProgress::Applied(ChainTip {
        height: next_height,
        hash: block_hash,
    }))
}

// --- Pending mempool synchronization ---

/// Captures a coherent mempool snapshot anchored to our current indexed tip.
///
/// The node tip and mempool sequence are checked both before and after fetching
/// transactions so we never publish a mix of two different mempool states.
pub async fn refresh_mempool(
    node: &NodeClient,
    database: &mut IndexDatabase,
    cache: &mut MempoolCache,
) -> Result<MempoolProgress> {
    let base = database
        .tip()?
        .ok_or_else(|| Error::Unavailable("genesis is not indexed".into()))?;

    let before_tip = node.fetch_tip().await?;
    let before_mempool = node.fetch_mempool_ids().await?;

    let tip_matches_base =
        before_tip.blocks == base.height && decode_hash(&before_tip.bestblockhash)? == base.hash;
    if !tip_matches_base {
        database.mark_unavailable()?;
        return Ok(MempoolProgress::ChangedDuringRead);
    }

    let sequence = i64::try_from(before_mempool.mempool_sequence)
        .map_err(|_| Error::InvalidData("mempool sequence exceeds signed 64-bit storage".into()))?;

    let mut pending_txids = Vec::with_capacity(before_mempool.txids.len());
    for txid_hex in &before_mempool.txids {
        pending_txids.push(decode_hash(txid_hex)?);
    }

    let (saved_base_hash, saved_sequence) = database.saved_mempool_state()?;
    let mempool_unchanged = saved_base_hash.as_deref() == Some(base.hash.as_slice())
        && saved_sequence == Some(sequence)
        && pending_txids.len() == cache.len()
        && pending_txids.iter().all(|id| cache.contains_key(id));

    let mut fetched_transactions = Vec::new();
    if !mempool_unchanged {
        database.mark_unavailable()?;

        for txid in &pending_txids {
            let transaction = match cache.get(txid) {
                Some(cached) => cached.clone(),
                None => node.fetch_pending_transaction(txid).await?,
            };
            fetched_transactions.push(transaction);
        }
    }

    let after_mempool = node.fetch_mempool_ids().await?;
    let after_tip = node.fetch_tip().await?;

    let changed_during_read = before_mempool != after_mempool
        || before_tip.bestblockhash != after_tip.bestblockhash
        || before_tip.blocks != after_tip.blocks;
    if changed_during_read {
        database.mark_unavailable()?;
        return Ok(MempoolProgress::ChangedDuringRead);
    }

    if mempool_unchanged {
        database.refresh_mempool_timestamp()?;
    } else {
        let ordered = order_parents_before_children(fetched_transactions)?;
        database.publish_mempool_snapshot(&base, before_mempool.mempool_sequence, &ordered)?;

        cache.clear();
        for transaction in ordered {
            cache.insert(transaction.txid, transaction);
        }
    }

    Ok(MempoolProgress::Ready)
}

/// Topologically sorts pending transactions so any unconfirmed parent appears
/// before the child transactions that spend its outputs.
fn order_parents_before_children(transactions: Vec<Transaction>) -> Result<Vec<Transaction>> {
    let mut by_txid = BTreeMap::new();
    for transaction in transactions {
        if by_txid.insert(transaction.txid, transaction).is_some() {
            return Err(Error::InvalidData(
                "duplicate pending transaction ID".into(),
            ));
        }
    }

    // Count how many unconfirmed parents each transaction waits on, and track
    // which children become unblocked when a parent is placed.
    let mut unplaced_parents = BTreeMap::new();
    let mut children_by_parent = BTreeMap::<Hash, Vec<Hash>>::new();
    let mut ready = BTreeSet::new();

    for (txid, transaction) in &by_txid {
        let mut parent_count = 0usize;
        for input in &transaction.inputs {
            if by_txid.contains_key(&input.txid) {
                parent_count += 1;
                children_by_parent
                    .entry(input.txid)
                    .or_default()
                    .push(*txid);
            }
        }

        unplaced_parents.insert(*txid, parent_count);
        if parent_count == 0 {
            ready.insert(*txid);
        }
    }

    let mut ordered = Vec::with_capacity(by_txid.len());
    while let Some(txid) = ready.pop_first() {
        let transaction = by_txid
            .remove(&txid)
            .ok_or_else(|| Error::InvalidData("pending dependency resolution failed".into()))?;
        ordered.push(transaction);

        if let Some(children) = children_by_parent.get(&txid) {
            for child in children {
                let remaining = unplaced_parents
                    .get_mut(child)
                    .ok_or_else(|| Error::InvalidData("missing pending dependency".into()))?;
                *remaining -= 1;
                if *remaining == 0 {
                    ready.insert(*child);
                }
            }
        }
    }

    if !by_txid.is_empty() {
        return Err(Error::InvalidData(
            "cyclic pending transaction dependencies".into(),
        ));
    }

    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use truenorth_rpc::{Hash, Outpoint, Transaction};

    use super::order_parents_before_children;
    use crate::test_support::hash;

    fn pending_tx(id: u64, parents: &[u64]) -> Transaction {
        let mut inputs = Vec::new();
        for parent in parents {
            inputs.push(Outpoint {
                txid: hash(*parent),
                vout: 0,
            });
        }
        Transaction {
            txid: hash(id),
            is_coinbase: false,
            inputs,
            outputs: Vec::new(),
        }
    }

    #[test]
    fn orders_unconfirmed_parents_before_children() {
        let grandparent = pending_tx(1, &[99]);
        let parent = pending_tx(2, &[1]);
        let child = pending_tx(3, &[2]);

        let ordered = order_parents_before_children(vec![child, parent, grandparent]).unwrap();
        let ids: Vec<Hash> = ordered.into_iter().map(|tx| tx.txid).collect();
        assert_eq!(ids, vec![hash(1), hash(2), hash(3)]);
    }

    #[test]
    fn rejects_duplicate_and_cyclic_pending_transactions() {
        let duplicate = order_parents_before_children(vec![pending_tx(1, &[]), pending_tx(1, &[])]);
        assert!(duplicate.is_err());

        let cycle = order_parents_before_children(vec![pending_tx(1, &[2]), pending_tx(2, &[1])]);
        assert!(cycle.is_err());
    }
}
