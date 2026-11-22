use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use shared::IndexStatus;

use crate::database::read_tip;
use crate::queries::{read_optional_u64, read_u64};
use crate::{Error, Result};

/// Index data is considered stale if the node or mempool was last observed
/// more than 30 seconds ago.
pub const MAX_INDEX_AGE_SECONDS: u64 = 30;

struct SavedStatus {
    observed_height: Option<u64>,
    observed_hash: Option<Vec<u8>>,
    last_node_contact: Option<u64>,
    ready: bool,
    pending_base_hash: Option<Vec<u8>>,
    pending_observed_at: Option<u64>,
    pending_revision: u64,
}

pub(crate) fn unix_time() -> Result<i64> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::InvalidData("system clock precedes Unix epoch".into()))?
        .as_secs();

    i64::try_from(seconds).map_err(|_| Error::InvalidData("timestamp overflow".into()))
}

/// Evaluates whether the index is caught up, coherent with its pending snapshot,
/// and fresh enough to serve wallet queries.
pub(crate) fn read_status(connection: &Connection) -> Result<IndexStatus> {
    let tip = read_tip(connection)?;
    let saved = connection.query_row(
        "SELECT observed_height, observed_hash, last_node_contact, ready,
                pending_base_hash, pending_observed_at, pending_revision
         FROM sync_status
         WHERE singleton = 1",
        [],
        |row| {
            Ok(SavedStatus {
                observed_height: read_optional_u64(row, 0)?,
                observed_hash: row.get(1)?,
                last_node_contact: read_optional_u64(row, 2)?,
                ready: row.get(3)?,
                pending_base_hash: row.get(4)?,
                pending_observed_at: read_optional_u64(row, 5)?,
                pending_revision: read_u64(row, 6)?,
            })
        },
    )?;

    let now = unix_time()? as u64;
    let tip_hash = tip.as_ref().map(|tip| tip.hash.as_slice());
    let tip_height = tip.as_ref().map(|tip| tip.height as u64);

    let pending_matches_tip = tip_hash.is_some() && saved.pending_base_hash.as_deref() == tip_hash;
    let pending_available =
        saved.ready && pending_matches_tip && is_fresh(saved.pending_observed_at, now);

    let node_matches_tip =
        saved.observed_hash.as_deref() == tip_hash && saved.observed_height == tip_height;
    let ready = pending_available && node_matches_tip && is_fresh(saved.last_node_contact, now);

    Ok(IndexStatus {
        ready,
        indexed_height: tip_height,
        indexed_hash: tip.map(|tip| hex::encode(tip.hash)),
        observed_node_height: saved.observed_height,
        observed_node_hash: saved.observed_hash.map(hex::encode),
        last_node_contact: saved.last_node_contact,
        pending_available,
        pending_observed_at: saved.pending_observed_at,
        pending_revision: Some(saved.pending_revision),
        max_age_seconds: MAX_INDEX_AGE_SECONDS,
    })
}

fn is_fresh(observed_at: Option<u64>, now: u64) -> bool {
    match observed_at {
        Some(time) => time <= now && now - time <= MAX_INDEX_AGE_SECONDS,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::read_status;
    use crate::test_support::Fixture;

    #[test]
    fn pending_revision_identifies_snapshots_even_at_the_same_chain_tip() {
        let mut fixture = Fixture::new(0);
        let base = fixture.database.tip().unwrap().unwrap();
        fixture
            .database
            .publish_mempool_snapshot(&base, 1, &[])
            .unwrap();
        let first = read_status(&fixture.reader()).unwrap();

        fixture
            .database
            .publish_mempool_snapshot(&base, 2, &[])
            .unwrap();
        let second = read_status(&fixture.reader()).unwrap();

        assert!(first.ready && second.ready);
        assert_eq!(first.indexed_hash, second.indexed_hash);
        assert_eq!(
            second.pending_revision,
            Some(first.pending_revision.unwrap() + 1)
        );
    }
}
