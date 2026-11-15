use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use shared::IndexStatus;
use truenorth_rpc::{NodeTip, decode_hash};

use crate::database::read_tip;
use crate::{Error, IndexDatabase, Result};

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

impl IndexDatabase {
    pub fn mark_unavailable(&self) -> Result<()> {
        self.connection
            .execute("UPDATE sync_status SET ready = 0", [])?;
        Ok(())
    }

    pub(crate) fn record_node_observation(&self, tip: &NodeTip) -> Result<()> {
        self.connection.execute(
            "UPDATE sync_status SET observed_height = ?1, observed_hash = ?2, last_node_contact = ?3",
            params![tip.blocks, decode_hash(&tip.bestblockhash)?.as_slice(), unix_time()?],
        )?;
        Ok(())
    }
}

pub(crate) fn read_status(connection: &Connection) -> Result<IndexStatus> {
    let tip = read_tip(connection)?;
    let status = connection.query_row(
        "SELECT observed_height, observed_hash, last_node_contact, ready,
            pending_base_hash, pending_observed_at, pending_revision
            FROM sync_status WHERE singleton = 1",
        [],
        |row| {
            Ok(SavedStatus {
                observed_height: crate::queries::read_optional_u64(row, 0)?,
                observed_hash: row.get(1)?,
                last_node_contact: crate::queries::read_optional_u64(row, 2)?,
                ready: row.get(3)?,
                pending_base_hash: row.get(4)?,
                pending_observed_at: crate::queries::read_optional_u64(row, 5)?,
                pending_revision: crate::queries::read_u64(row, 6)?,
            })
        },
    )?;
    let now = unix_time()? as u64;
    let fresh = |time: Option<u64>| {
        time.is_some_and(|time| time <= now && now - time <= MAX_INDEX_AGE_SECONDS)
    };
    let tip_hash = tip.as_ref().map(|tip| tip.hash.as_slice());
    let pending_available = status.ready
        && tip_hash.is_some()
        && status.pending_base_hash.as_deref() == tip_hash
        && fresh(status.pending_observed_at);
    let ready = pending_available
        && status.observed_hash.as_deref() == tip_hash
        && status.observed_height == tip.as_ref().map(|tip| tip.height as u64)
        && fresh(status.last_node_contact);
    Ok(IndexStatus {
        ready,
        indexed_height: tip.as_ref().map(|tip| tip.height as u64),
        indexed_hash: tip.map(|tip| hex::encode(tip.hash)),
        observed_node_height: status.observed_height,
        observed_node_hash: status.observed_hash.map(hex::encode),
        last_node_contact: status.last_node_contact,
        pending_available,
        pending_observed_at: status.pending_observed_at,
        pending_revision: Some(status.pending_revision),
        max_age_seconds: MAX_INDEX_AGE_SECONDS,
    })
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
