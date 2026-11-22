//! SQLite blockchain indexer and read-only wallet query layer for TrueNorth.
//!
//! The crate is organized around four responsibilities:
//! - [`address`]: convert TrueNorth addresses to and from script bytes.
//! - [`database`] & [`status`]: own the SQLite writer connection, schema, and readiness state.
//! - [`sync`]: pull confirmed blocks and mempool snapshots from `truenorthd`.
//! - [`queries`], [`utxos`], & [`history`]: serve consistent read-only snapshots to the API server.

mod address;
mod database;
mod error;
mod history;
mod queries;
mod status;
mod sync;
mod utxos;

#[cfg(test)]
mod test_support;

pub use address::{ScriptAddress, address_from_script, decode_addresses};
pub use database::{ChainTip, IndexDatabase};
pub use error::{Error, Result};
pub use queries::{IndexReader, ReadSnapshot};
pub use sync::{
    MempoolCache, MempoolProgress, SyncProgress, refresh_mempool, run_worker,
    synchronize_next_block,
};
pub use truenorth_rpc::{Block, Hash, decode_block, decode_hash};
