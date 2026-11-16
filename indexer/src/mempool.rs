use std::collections::{BTreeMap, BTreeSet};

use truenorth_rpc::{Hash, NodeClient, Transaction, decode_hash};

use crate::status::unix_time;
use crate::{Error, IndexDatabase, Result};

pub type MempoolCache = BTreeMap<Hash, Transaction>;

pub enum MempoolProgress {
    Ready,
    ChangedDuringRead,
}

pub async fn refresh_mempool(
    node: &NodeClient,
    database: &mut IndexDatabase,
    cache: &mut MempoolCache,
) -> Result<MempoolProgress> {
    let base = database
        .tip()?
        .ok_or_else(|| Error::Unavailable("genesis is not indexed".into()))?;
    let before_tip = node.fetch_tip().await?;
    let before = node.fetch_mempool_ids().await?;
    if before_tip.blocks != base.height || decode_hash(&before_tip.bestblockhash)? != base.hash {
        database.mark_unavailable()?;
        return Ok(MempoolProgress::ChangedDuringRead);
    }
    let saved: (Option<Vec<u8>>, Option<i64>) = database.connection.query_row(
        "SELECT pending_base_hash, pending_sequence FROM sync_status",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let sequence = i64::try_from(before.mempool_sequence)
        .map_err(|_| Error::InvalidData("mempool sequence exceeds signed 64-bit storage".into()))?;
    let unchanged = saved.0.as_deref() == Some(base.hash.as_slice())
        && saved.1 == Some(sequence)
        && before.txids.len() == cache.len()
        && before
            .txids
            .iter()
            .all(|id| decode_hash(id).is_ok_and(|id| cache.contains_key(&id)));
    if !unchanged {
        database.mark_unavailable()?;
    }
    let mut transactions = Vec::new();
    if !unchanged {
        for id in &before.txids {
            let id = decode_hash(id)?;
            let transaction = match cache.get(&id) {
                Some(transaction) => transaction.clone(),
                None => node.fetch_pending_transaction(&id).await?,
            };
            transactions.push(transaction);
        }
    }
    let after = node.fetch_mempool_ids().await?;
    let after_tip = node.fetch_tip().await?;
    if before != after
        || before_tip.bestblockhash != after_tip.bestblockhash
        || before_tip.blocks != after_tip.blocks
    {
        database.mark_unavailable()?;
        return Ok(MempoolProgress::ChangedDuringRead);
    }
    if unchanged {
        database.connection.execute(
            "UPDATE sync_status SET ready = 1, pending_observed_at = ?1, last_node_contact = ?1",
            [unix_time()?],
        )?;
    } else {
        let ordered = order_transactions(transactions)?;
        database.publish_mempool_snapshot(&base, before.mempool_sequence, &ordered)?;
        cache.clear();
        for transaction in ordered {
            cache.insert(transaction.txid, transaction);
        }
    }
    Ok(MempoolProgress::Ready)
}

fn order_transactions(transactions: Vec<Transaction>) -> Result<Vec<Transaction>> {
    let mut remaining = BTreeMap::new();
    for transaction in transactions {
        if remaining.insert(transaction.txid, transaction).is_some() {
            return Err(Error::InvalidData(
                "duplicate pending transaction ID".into(),
            ));
        }
    }
    let mut dependencies = BTreeMap::new();
    let mut children = BTreeMap::<Hash, Vec<Hash>>::new();
    let mut ready = BTreeSet::new();
    for (id, transaction) in &remaining {
        let mut count = 0;
        for input in &transaction.inputs {
            if remaining.contains_key(&input.txid) {
                count += 1;
                children.entry(input.txid).or_default().push(*id);
            }
        }
        dependencies.insert(*id, count);
        if count == 0 {
            ready.insert(*id);
        }
    }
    let mut ordered = Vec::new();
    while let Some(id) = ready.pop_first() {
        let transaction = remaining
            .remove(&id)
            .ok_or_else(|| Error::InvalidData("pending dependency resolution failed".into()))?;
        ordered.push(transaction);
        if let Some(children) = children.get(&id) {
            for child in children {
                let count = dependencies
                    .get_mut(child)
                    .ok_or_else(|| Error::InvalidData("missing pending dependency".into()))?;
                *count -= 1;
                if *count == 0 {
                    ready.insert(*child);
                }
            }
        }
    }
    if !remaining.is_empty() {
        return Err(Error::InvalidData(
            "cyclic pending transaction dependencies".into(),
        ));
    }
    Ok(ordered)
}
