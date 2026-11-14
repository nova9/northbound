use serde::Deserialize;
use serde_json::Number;

use crate::amount::{MAX_MONEY, atomic_units};
use crate::{Error, Result};

pub type Hash = [u8; 32];

#[derive(Debug)]
pub struct Block {
    pub hash: Hash,
    pub height: i64,
    pub previous_hash: Option<Hash>,
    pub timestamp: i64,
    pub transactions: Vec<Transaction>,
}

impl Block {
    pub fn validate(&self) -> Result<()> {
        if self.height < 0
            || self.timestamp < 0
            || self.transactions.is_empty()
            || (self.height == 0) != self.previous_hash.is_none()
        {
            return Err(Error::InvalidData("invalid block metadata".into()));
        }
        for (position, transaction) in self.transactions.iter().enumerate() {
            if transaction.is_coinbase != (position == 0)
                || transaction.outputs.is_empty()
                || transaction.is_coinbase != transaction.inputs.is_empty()
            {
                return Err(Error::InvalidData(
                    "invalid transaction position or input/output count".into(),
                ));
            }
            let mut total = 0_i64;
            for output in &transaction.outputs {
                if output.amount < 0 {
                    return Err(Error::InvalidData("negative output amount".into()));
                }
                total = total
                    .checked_add(output.amount)
                    .filter(|total| *total <= MAX_MONEY)
                    .ok_or_else(|| Error::InvalidData("output total exceeds MAX_MONEY".into()))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Transaction {
    pub txid: Hash,
    pub is_coinbase: bool,
    pub inputs: Vec<Outpoint>,
    pub outputs: Vec<Output>,
}

#[derive(Clone, Debug)]
pub struct Outpoint {
    pub txid: Hash,
    pub vout: u32,
}

#[derive(Clone, Debug)]
pub struct Output {
    pub amount: i64,
    pub script: Vec<u8>,
}

#[derive(Deserialize)]
struct RpcBlock {
    hash: String,
    height: i64,
    previousblockhash: Option<String>,
    time: i64,
    tx: Vec<RpcTransaction>,
}

#[derive(Deserialize)]
struct RpcTransaction {
    txid: String,
    vin: Vec<RpcInput>,
    vout: Vec<RpcOutput>,
}

#[derive(Deserialize)]
struct RpcInput {
    coinbase: Option<String>,
    txid: Option<String>,
    vout: Option<u32>,
}

#[derive(Deserialize)]
struct RpcOutput {
    n: u32,
    value: Number,
    #[serde(rename = "scriptPubKey")]
    script_pubkey: RpcScript,
}

#[derive(Deserialize)]
struct RpcScript {
    hex: String,
}

pub fn decode_hash(text: &str) -> Result<Hash> {
    let mut bytes = [0; 32];
    hex::decode_to_slice(text, &mut bytes)
        .map_err(|_| Error::InvalidData("expected a 32-byte hexadecimal hash".into()))?;
    Ok(bytes)
}

pub fn decode_block(json: &str) -> Result<Block> {
    let rpc: RpcBlock = serde_json::from_str(json)
        .map_err(|error| Error::InvalidData(format!("cannot decode block: {error}")))?;
    if rpc.height < 0 || rpc.time < 0 || rpc.tx.is_empty() {
        return Err(Error::InvalidData(
            "invalid block height, time, or transaction count".into(),
        ));
    }
    let previous_hash = match rpc.previousblockhash {
        Some(hash) => Some(decode_hash(&hash)?),
        None => None,
    };
    if (rpc.height == 0) != previous_hash.is_none() {
        return Err(Error::InvalidData(
            "only genesis may have no previous hash".into(),
        ));
    }

    let mut transactions = Vec::new();
    for (position, rpc_transaction) in rpc.tx.into_iter().enumerate() {
        let transaction = decode_rpc_transaction(rpc_transaction)?;
        if transaction.is_coinbase != (position == 0) {
            return Err(Error::InvalidData(
                "only the first transaction must be coinbase".into(),
            ));
        }
        transactions.push(transaction);
    }
    Ok(Block {
        hash: decode_hash(&rpc.hash)?,
        height: rpc.height,
        previous_hash,
        timestamp: rpc.time,
        transactions,
    })
}

pub fn decode_transaction(json: &str) -> Result<Transaction> {
    let rpc = serde_json::from_str(json)
        .map_err(|error| Error::InvalidData(format!("cannot decode transaction: {error}")))?;
    decode_rpc_transaction(rpc)
}

fn decode_rpc_transaction(rpc: RpcTransaction) -> Result<Transaction> {
    if rpc.vin.is_empty() || rpc.vout.is_empty() {
        return Err(Error::InvalidData(
            "transaction must have inputs and outputs".into(),
        ));
    }
    let is_coinbase = rpc.vin.len() == 1 && rpc.vin[0].coinbase.is_some();
    let mut inputs = Vec::new();
    for input in rpc.vin {
        if is_coinbase {
            if input.txid.is_some() || input.vout.is_some() {
                return Err(Error::InvalidData(
                    "coinbase input also contains an outpoint".into(),
                ));
            }
            continue;
        }
        let (Some(txid), Some(vout), None) = (input.txid, input.vout, input.coinbase) else {
            return Err(Error::InvalidData(
                "ordinary input must identify a previous outpoint".into(),
            ));
        };
        inputs.push(Outpoint {
            txid: decode_hash(&txid)?,
            vout,
        });
    }
    let mut outputs = Vec::new();
    let mut total = 0_i64;
    for (position, output) in rpc.vout.into_iter().enumerate() {
        if usize::try_from(output.n).ok() != Some(position) {
            return Err(Error::InvalidData(
                "output indexes must be consecutive from zero".into(),
            ));
        }
        let amount = atomic_units(&output.value)?;
        total = total
            .checked_add(amount)
            .filter(|total| *total <= MAX_MONEY)
            .ok_or_else(|| {
                Error::InvalidData("transaction output total exceeds MAX_MONEY".into())
            })?;
        let script = hex::decode(output.script_pubkey.hex)
            .map_err(|_| Error::InvalidData("output script is not hexadecimal".into()))?;
        outputs.push(Output { amount, script });
    }
    Ok(Transaction {
        txid: decode_hash(&rpc.txid)?,
        is_coinbase,
        inputs,
        outputs,
    })
}
