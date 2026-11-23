use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use bitcoin::hex::FromHex;
use bitcoin::{Amount, OutPoint, ScriptBuf, Txid};
use shared::WalletUtxo;

use crate::crypto::{keys, p2qrh};

#[derive(Clone, Debug)]
pub struct VerifiedUtxo {
    pub outpoint: OutPoint,
    pub amount: Amount,
    pub script_pubkey: ScriptBuf,
    pub keychain: WalletKeychain,
    pub derivation_index: u32,
    pub confirmations: u64,
    pub coinbase: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalletKeychain {
    Receiving,
    Change,
}

impl VerifiedUtxo {
    pub fn is_spendable(&self) -> bool {
        let coinbase_is_mature =
            !self.coinbase || self.confirmations >= shared::COINBASE_MATURITY_CONFIRMATIONS;

        self.confirmations > 0 && coinbase_is_mature
    }
}

pub fn calculate_spendable_balance(utxos: &[VerifiedUtxo]) -> Result<u64, String> {
    let mut balance = 0_u64;

    for utxo in utxos {
        if !utxo.is_spendable() {
            continue;
        }

        balance = balance
            .checked_add(utxo.amount.to_sat())
            .ok_or_else(|| "wallet balance is too large".to_owned())?;
    }

    Ok(balance)
}

pub(crate) fn verify_utxos(
    public_keys: &keys::AccountPublicKeys,
    utxos: Vec<WalletUtxo>,
    receiving_indices: &[u32],
    change_count: usize,
) -> Result<Vec<VerifiedUtxo>, String> {
    let mut derivation_by_script = HashMap::new();

    for &index in receiving_indices {
        let public_key = public_keys.receiving_public_key(index)?;
        let script_pubkey = p2qrh::script_pubkey_from_public_key(&public_key).to_vec();
        derivation_by_script.insert(script_pubkey, (WalletKeychain::Receiving, index));
    }

    for index in 0..change_count {
        let index = u32::try_from(index)
            .map_err(|_| "wallet change address index is exhausted".to_owned())?;
        let public_key = public_keys.change_public_key(index)?;
        let script_pubkey = p2qrh::script_pubkey_from_public_key(&public_key).to_vec();
        derivation_by_script.insert(script_pubkey, (WalletKeychain::Change, index));
    }

    let mut seen_outpoints = HashSet::new();
    let mut verified_utxos = Vec::with_capacity(utxos.len());

    for utxo in utxos {
        if utxo.amount == 0 {
            return Err("server returned a zero-value UTXO".to_owned());
        }

        let transaction_id = Txid::from_str(&utxo.transaction_id)
            .map_err(|_| "server returned an invalid UTXO transaction ID".to_owned())?;
        let outpoint = OutPoint::new(transaction_id, utxo.output_index);

        if !seen_outpoints.insert(outpoint) {
            return Err("server returned a duplicate UTXO".to_owned());
        }

        let script_bytes = Vec::<u8>::from_hex(&utxo.script_pubkey)
            .map_err(|_| "server returned an invalid UTXO script".to_owned())?;
        let (keychain, derivation_index) = derivation_by_script
            .get(&script_bytes)
            .copied()
            .ok_or_else(|| "server returned a UTXO not owned by this wallet".to_owned())?;

        verified_utxos.push(VerifiedUtxo {
            outpoint,
            amount: Amount::from_sat(utxo.amount),
            script_pubkey: ScriptBuf::from_bytes(script_bytes),
            keychain,
            derivation_index,
            confirmations: utxo.confirmations,
            coinbase: utxo.coinbase,
        });
    }

    Ok(verified_utxos)
}

#[cfg(test)]
mod tests {
    use shared::WalletUtxo;

    use super::{VerifiedUtxo, WalletKeychain, calculate_spendable_balance, verify_utxos};
    use crate::crypto::{keys, p2qrh};

    #[test]
    fn mining_rewards_enter_spendable_balance_at_one_hundred_confirmations() {
        let mut reward = VerifiedUtxo {
            outpoint: bitcoin::OutPoint::null(),
            amount: bitcoin::Amount::from_sat(51_200_000_000),
            script_pubkey: bitcoin::ScriptBuf::new(),
            keychain: WalletKeychain::Receiving,
            derivation_index: 0,
            confirmations: 68,
            coinbase: true,
        };
        assert_eq!(calculate_spendable_balance(&[reward.clone()]).unwrap(), 0);
        reward.confirmations = 99;
        assert!(!reward.is_spendable());
        reward.confirmations = 100;
        assert_eq!(
            calculate_spendable_balance(&[reward]).unwrap(),
            51_200_000_000
        );
    }

    #[test]
    fn utxo_is_mapped_to_its_receiving_derivation_index() {
        let public_keys = keys::AccountPublicKeys::from_entropy(&[0_u8; 16]).expect("keys");
        let public_key = keys::derive_receiving_public_key_from_entropy(&[0_u8; 16], 0)
            .expect("test key should derive");
        let script_pubkey = hex::encode(p2qrh::script_pubkey_from_public_key(&public_key));
        let server_utxo = WalletUtxo {
            transaction_id: "fd2344a47141b786846d7b9a78b1c76ed62375a27556f78e7e85bc128056ba4e"
                .to_owned(),
            output_index: 0,
            amount: 1_000_000_000,
            script_pubkey,
            block_height: 104,
            confirmations: 1,
            coinbase: false,
        };

        let verified = verify_utxos(&public_keys, vec![server_utxo], &[0], 1)
            .expect("wallet should recognize its output");

        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].keychain, WalletKeychain::Receiving);
        assert_eq!(verified[0].derivation_index, 0);
        assert_eq!(verified[0].amount.to_sat(), 1_000_000_000);
    }

    #[test]
    fn utxo_is_mapped_to_its_change_derivation_index() {
        let public_keys = keys::AccountPublicKeys::from_entropy(&[0_u8; 16]).expect("keys");
        let public_key = keys::derive_change_public_key_from_entropy(&[0_u8; 16], 0)
            .expect("test change key should derive");
        let script_pubkey = hex::encode(p2qrh::script_pubkey_from_public_key(&public_key));
        let server_utxo = WalletUtxo {
            transaction_id: "97e195eafc5824409588218a47ccf75e4377bc43555ffb30f80250c1f24124ff"
                .to_owned(),
            output_index: 1,
            amount: 899_998_360,
            script_pubkey,
            block_height: 105,
            confirmations: 1,
            coinbase: false,
        };

        let verified = verify_utxos(&public_keys, vec![server_utxo], &[0], 1)
            .expect("wallet should recognize its change output");

        assert_eq!(verified[0].keychain, WalletKeychain::Change);
        assert_eq!(verified[0].derivation_index, 0);
        assert_eq!(verified[0].amount.to_sat(), 899_998_360);
    }
}
