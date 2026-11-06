use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;

use bitcoin::hex::FromHex;
use bitcoin::{Amount, OutPoint, ScriptBuf, Transaction, Txid};
use shared::WalletUtxo;

use crate::config::{WALLET_NETWORK, wallet_path};
use crate::keys;
use crate::p2qrh;
use crate::wallet_file;
use crate::wallet_secrets::{
    DecryptedWalletSecrets, EncryptedWalletSecrets, EncryptionKey, SALT_LENGTH,
};

const RECOVERY_GAP_LIMIT: u32 = 20;
const MAX_CONSECUTIVE_UNUSED_ADDRESSES: usize = (RECOVERY_GAP_LIMIT - 1) as usize;

pub struct ReceivingAddress {
    pub index: u32,
    pub address: String,
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveredAddressIndexes {
    pub next_receiving_index: u32,
    pub next_change_index: u32,
}

impl VerifiedUtxo {
    pub fn is_spendable(&self) -> bool {
        let coinbase_is_mature = !self.coinbase || self.confirmations >= 100;

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

pub struct UnlockedWallet {
    secrets: DecryptedWalletSecrets,
    encryption_key: EncryptionKey,
    salt: [u8; SALT_LENGTH],
}

impl UnlockedWallet {
    fn from_encrypted_wallet(
        encrypted_wallet: &EncryptedWalletSecrets,
        password: &str,
    ) -> Result<Self, String> {
        let encryption_key = encrypted_wallet.derive_encryption_key(password)?;
        let secrets = encrypted_wallet.decrypt_wallet_with_key(&encryption_key)?;
        let salt = *encrypted_wallet.salt();

        Ok(Self {
            secrets,
            encryption_key,
            salt,
        })
    }

    pub fn receiving_addresses(&self) -> Result<Vec<ReceivingAddress>, String> {
        let next_index = self.secrets.next_receiving_index();
        let mut addresses = Vec::new();

        for index in 0..next_index {
            let address = self.receiving_address_at(index)?;

            addresses.push(ReceivingAddress { index, address });
        }

        Ok(addresses)
    }

    pub fn confirms_recovery_phrase(&self, recovery_phrase: &str) -> Result<bool, String> {
        keys::recovery_phrase_matches_entropy(recovery_phrase, self.secrets.bip39_entropy())
    }

    pub fn tracked_addresses(&self) -> Result<Vec<String>, String> {
        let mut addresses = Vec::new();

        for index in 0..self.secrets.next_receiving_index() {
            addresses.push(self.receiving_address_at(index)?);
        }

        for index in 0..self.secrets.next_change_index() {
            let public_key =
                keys::derive_change_public_key_from_entropy(self.secrets.bip39_entropy(), index)?;
            addresses.push(p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)?);
        }

        Ok(addresses)
    }

    pub fn receiving_address_at(&self, index: u32) -> Result<String, String> {
        let public_key =
            keys::derive_receiving_public_key_from_entropy(self.secrets.bip39_entropy(), index)?;

        p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)
    }

    pub fn recent_receiving_addresses_for_allocation_check(&self) -> Result<Vec<String>, String> {
        let next_index = self.secrets.next_receiving_index();
        let addresses_to_check = u32::try_from(MAX_CONSECUTIVE_UNUSED_ADDRESSES)
            .map_err(|_| "receiving address limit is too large".to_owned())?;
        let first_index = next_index.saturating_sub(addresses_to_check);
        let mut addresses = Vec::new();

        for index in first_index..next_index {
            addresses.push(self.receiving_address_at(index)?);
        }

        Ok(addresses)
    }

    pub fn can_allocate_receiving_address(
        &self,
        checked_addresses: &[String],
        used_addresses: &[String],
    ) -> Result<bool, String> {
        let current_addresses = self.recent_receiving_addresses_for_allocation_check()?;

        if checked_addresses != current_addresses {
            return Err("receiving addresses changed while activity was checked".to_owned());
        }

        let checked_addresses = checked_addresses.iter().collect::<HashSet<_>>();

        for used_address in used_addresses {
            if !checked_addresses.contains(used_address) {
                return Err(
                    "wallet server returned activity for an address not requested".to_owned(),
                );
            }
        }

        if checked_addresses.len() < MAX_CONSECUTIVE_UNUSED_ADDRESSES {
            return Ok(true);
        }

        Ok(!used_addresses.is_empty())
    }

    pub async fn scan_recovery_address_indexes(&self) -> Result<RecoveredAddressIndexes, String> {
        let next_receiving_index = self
            .scan_keychain_for_last_used_address(WalletKeychain::Receiving)
            .await?;
        let next_change_index = self
            .scan_keychain_for_last_used_address(WalletKeychain::Change)
            .await?;

        Ok(RecoveredAddressIndexes {
            next_receiving_index,
            next_change_index,
        })
    }

    async fn scan_keychain_for_last_used_address(
        &self,
        keychain: WalletKeychain,
    ) -> Result<u32, String> {
        let mut batch_start = 0_u32;
        let mut last_used_index = None;

        loop {
            let mut addresses = Vec::with_capacity(RECOVERY_GAP_LIMIT as usize);

            for offset in 0..RECOVERY_GAP_LIMIT {
                let index = batch_start
                    .checked_add(offset)
                    .ok_or_else(|| "wallet address index is exhausted".to_owned())?;
                addresses.push(self.address_at(keychain, index)?);
            }

            let used_addresses = crate::api::fetch_used_addresses(addresses.clone()).await?;
            let requested_addresses = addresses.iter().cloned().collect::<HashSet<_>>();

            for address in &used_addresses {
                if !requested_addresses.contains(address) {
                    return Err(
                        "wallet server returned activity for an address not requested".to_owned(),
                    );
                }
            }

            for (offset, address) in addresses.iter().enumerate() {
                if used_addresses.contains(address) {
                    let offset = u32::try_from(offset)
                        .map_err(|_| "wallet recovery batch is too large".to_owned())?;
                    last_used_index = Some(
                        batch_start
                            .checked_add(offset)
                            .ok_or_else(|| "wallet address index is exhausted".to_owned())?,
                    );
                }
            }

            let batch_end = batch_start
                .checked_add(RECOVERY_GAP_LIMIT - 1)
                .ok_or_else(|| "wallet address index is exhausted".to_owned())?;
            let unused_after_last_used = match last_used_index {
                Some(index) => batch_end - index,
                None => batch_end + 1,
            };

            if unused_after_last_used >= RECOVERY_GAP_LIMIT {
                return match last_used_index {
                    Some(index) => index
                        .checked_add(1)
                        .ok_or_else(|| "wallet address index is exhausted".to_owned()),
                    None => Ok(0),
                };
            }

            batch_start = batch_start
                .checked_add(RECOVERY_GAP_LIMIT)
                .ok_or_else(|| "wallet address index is exhausted".to_owned())?;
        }
    }

    fn address_at(&self, keychain: WalletKeychain, index: u32) -> Result<String, String> {
        let public_key = match keychain {
            WalletKeychain::Receiving => {
                keys::derive_receiving_public_key_from_entropy(self.secrets.bip39_entropy(), index)?
            }
            WalletKeychain::Change => {
                keys::derive_change_public_key_from_entropy(self.secrets.bip39_entropy(), index)?
            }
        };

        p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)
    }

    pub fn persist_recovered_address_indexes(
        &mut self,
        indexes: RecoveredAddressIndexes,
    ) -> Result<(), String> {
        let mut updated_secrets = self.secrets.clone();
        updated_secrets
            .set_next_address_indexes(indexes.next_receiving_index, indexes.next_change_index);

        let encrypted_wallet =
            updated_secrets.encrypt_with_key(&self.salt, &self.encryption_key)?;
        save_encrypted_wallet(&encrypted_wallet)?;
        self.secrets = updated_secrets;

        Ok(())
    }

    pub fn allocate_and_persist_next_receiving_address(&mut self) -> Result<String, String> {
        let index = self.secrets.next_receiving_index();
        let address = self.receiving_address_at(index)?;

        let mut updated_secrets = self.secrets.clone();
        updated_secrets.advance_receiving_index()?;

        let encrypted_wallet =
            updated_secrets.encrypt_with_key(&self.salt, &self.encryption_key)?;
        save_encrypted_wallet(&encrypted_wallet)?;

        self.secrets = updated_secrets;

        Ok(address)
    }

    pub fn next_change_address(&self) -> Result<String, String> {
        let index = self.secrets.next_change_index();
        let public_key =
            keys::derive_change_public_key_from_entropy(self.secrets.bip39_entropy(), index)?;

        p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)
    }

    pub fn reserve_and_persist_next_change_address(&mut self) -> Result<String, String> {
        let address = self.next_change_address()?;
        let mut updated_secrets = self.secrets.clone();
        updated_secrets.advance_change_index()?;

        let encrypted_wallet =
            updated_secrets.encrypt_with_key(&self.salt, &self.encryption_key)?;
        save_encrypted_wallet(&encrypted_wallet)?;

        self.secrets = updated_secrets;

        Ok(address)
    }

    pub fn verify_utxos(&self, utxos: Vec<WalletUtxo>) -> Result<Vec<VerifiedUtxo>, String> {
        let mut derivation_by_script = HashMap::new();

        for index in 0..self.secrets.next_receiving_index() {
            let public_key = keys::derive_receiving_public_key_from_entropy(
                self.secrets.bip39_entropy(),
                index,
            )?;
            let script_pubkey = p2qrh::script_pubkey_from_public_key(&public_key).to_vec();
            derivation_by_script.insert(script_pubkey, (WalletKeychain::Receiving, index));
        }

        for index in 0..self.secrets.next_change_index() {
            let public_key =
                keys::derive_change_public_key_from_entropy(self.secrets.bip39_entropy(), index)?;
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

    pub fn sign_p2qrh_transaction(
        &self,
        transaction: &mut Transaction,
        selected_utxos: &[VerifiedUtxo],
    ) -> Result<(), String> {
        crate::transaction_signing::sign_p2qrh_transaction(
            transaction,
            selected_utxos,
            self.secrets.bip39_entropy(),
        )
    }

    pub fn change_password(
        &mut self,
        current_password: &str,
        new_password: &str,
        new_password_confirmation: &str,
    ) -> Result<(), String> {
        self.change_password_at_path(
            &wallet_path()?,
            current_password,
            new_password,
            new_password_confirmation,
        )
    }

    fn change_password_at_path(
        &mut self,
        wallet_path: &Path,
        current_password: &str,
        new_password: &str,
        new_password_confirmation: &str,
    ) -> Result<(), String> {
        if current_password.is_empty() {
            return Err("current wallet password is required".to_owned());
        }

        validate_new_password(new_password, new_password_confirmation)?;

        if current_password == new_password {
            return Err(
                "new wallet password must be different from the current password".to_owned(),
            );
        }

        let encrypted_wallet = wallet_file::load(wallet_path)?;
        let current_encryption_key = encrypted_wallet.derive_encryption_key(current_password)?;
        let saved_secrets = encrypted_wallet.decrypt_wallet_with_key(&current_encryption_key)?;

        let saved_wallet_matches = saved_secrets.matches_entropy(self.secrets.bip39_entropy())
            && saved_secrets.next_receiving_index() == self.secrets.next_receiving_index()
            && saved_secrets.next_change_index() == self.secrets.next_change_index();

        if !saved_wallet_matches {
            return Err(
                "wallet file changed; unlock the wallet again before changing its password"
                    .to_owned(),
            );
        }

        let updated_encrypted_wallet = self.secrets.encrypt(new_password)?;
        let updated_encryption_key =
            updated_encrypted_wallet.derive_encryption_key(new_password)?;
        let updated_salt = *updated_encrypted_wallet.salt();

        wallet_file::save(wallet_path, &updated_encrypted_wallet)?;

        self.encryption_key = updated_encryption_key;
        self.salt = updated_salt;

        Ok(())
    }
}

pub fn save_new_wallet_and_unlock(
    pending_wallet: &keys::PendingWallet,
    entered_phrase: &str,
    password: &str,
    password_confirmation: &str,
) -> Result<UnlockedWallet, String> {
    let phrase_matches = pending_wallet.confirms_recovery_phrase(entered_phrase)?;

    if !phrase_matches {
        return Err("recovery phrase does not match".to_owned());
    }

    if password.chars().count() < 12 {
        return Err("wallet password must contain at least 12 characters".to_owned());
    }

    if password != password_confirmation {
        return Err("wallet passwords do not match".to_owned());
    }

    let wallet_path = wallet_path()?;

    if wallet_path.exists() {
        return Err(format!(
            "a wallet already exists at {}",
            wallet_path.display()
        ));
    }

    let encrypted_wallet = pending_wallet.encrypt(password)?;
    let unlocked_wallet = UnlockedWallet::from_encrypted_wallet(&encrypted_wallet, password)?;

    create_wallet_directory(&wallet_path)?;
    wallet_file::save(&wallet_path, &encrypted_wallet)?;

    Ok(unlocked_wallet)
}

pub fn load_and_unlock_wallet(password: &str) -> Result<UnlockedWallet, String> {
    let encrypted_wallet = wallet_file::load(&wallet_path()?)?;

    UnlockedWallet::from_encrypted_wallet(&encrypted_wallet, password)
}

pub fn prepare_restored_wallet(
    recovery_phrase: &str,
    password: &str,
    password_confirmation: &str,
) -> Result<UnlockedWallet, String> {
    validate_new_password(password, password_confirmation)?;

    let restored_wallet = keys::restore_wallet(recovery_phrase)?;

    let wallet_path = wallet_path()?;

    if wallet_path.exists() {
        return Err(format!(
            "a wallet already exists at {}",
            wallet_path.display()
        ));
    }

    let encrypted_wallet = restored_wallet.encrypt(password)?;
    UnlockedWallet::from_encrypted_wallet(&encrypted_wallet, password)
}

fn save_encrypted_wallet(encrypted_wallet: &EncryptedWalletSecrets) -> Result<(), String> {
    let wallet_path = wallet_path()?;
    create_wallet_directory(&wallet_path)?;

    wallet_file::save(&wallet_path, encrypted_wallet)
}

fn create_wallet_directory(wallet_path: &Path) -> Result<(), String> {
    let wallet_directory = wallet_path
        .parent()
        .ok_or_else(|| "wallet path does not contain a directory".to_owned())?;

    std::fs::create_dir_all(wallet_directory)
        .map_err(|error| format!("could not create wallet directory: {error}"))
}

fn validate_new_password(password: &str, password_confirmation: &str) -> Result<(), String> {
    if password.chars().count() < 12 {
        return Err("wallet password must contain at least 12 characters".to_owned());
    }

    if password != password_confirmation {
        return Err("wallet passwords do not match".to_owned());
    }

    Ok(())
}

pub fn exists() -> bool {
    wallet_path().is_ok_and(|path| path.exists())
}

#[cfg(test)]
mod tests {
    use shared::WalletUtxo;
    use zeroize::Zeroizing;

    use super::{UnlockedWallet, WalletKeychain};
    use crate::keys;
    use crate::p2qrh;
    use crate::wallet_secrets::DecryptedWalletSecrets;

    #[test]
    fn unlocked_wallet_confirms_its_recovery_phrase() {
        let pending_wallet = keys::generate_new_wallet().expect("wallet should be generated");
        let recovery_phrase = pending_wallet.recovery_phrase().to_owned();
        let encrypted_wallet = pending_wallet
            .encrypt("a secure test password")
            .expect("wallet should be encrypted");
        let unlocked_wallet =
            UnlockedWallet::from_encrypted_wallet(&encrypted_wallet, "a secure test password")
                .expect("wallet should unlock");

        let phrase_matches = unlocked_wallet
            .confirms_recovery_phrase(&recovery_phrase)
            .expect("recovery phrase should be valid");

        assert!(phrase_matches);
    }

    #[test]
    fn unlocked_wallet_rejects_another_recovery_phrase() {
        let pending_wallet = keys::generate_new_wallet().expect("wallet should be generated");
        let other_wallet = keys::generate_new_wallet().expect("second wallet should be generated");
        let other_recovery_phrase = other_wallet.recovery_phrase().to_owned();
        let encrypted_wallet = pending_wallet
            .encrypt("a secure test password")
            .expect("wallet should be encrypted");
        let unlocked_wallet =
            UnlockedWallet::from_encrypted_wallet(&encrypted_wallet, "a secure test password")
                .expect("wallet should unlock");

        let phrase_matches = unlocked_wallet
            .confirms_recovery_phrase(&other_recovery_phrase)
            .expect("recovery phrase should be valid");

        assert!(!phrase_matches);
    }

    #[test]
    fn utxo_is_mapped_to_its_receiving_derivation_index() {
        let mut secrets = DecryptedWalletSecrets::new([0_u8; 16]);
        secrets
            .advance_receiving_index()
            .expect("first receiving address should be allocated");

        let wallet = UnlockedWallet {
            secrets,
            encryption_key: Zeroizing::new([7_u8; 32]),
            salt: [9_u8; 16],
        };

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

        let verified = wallet
            .verify_utxos(vec![server_utxo])
            .expect("wallet should recognize its output");

        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].keychain, WalletKeychain::Receiving);
        assert_eq!(verified[0].derivation_index, 0);
        assert_eq!(verified[0].amount.to_sat(), 1_000_000_000);
    }

    #[test]
    fn utxo_is_mapped_to_its_change_derivation_index() {
        let mut secrets = DecryptedWalletSecrets::new([0_u8; 16]);
        secrets
            .advance_change_index()
            .expect("first change address should be reserved");

        let wallet = UnlockedWallet {
            secrets,
            encryption_key: Zeroizing::new([7_u8; 32]),
            salt: [9_u8; 16],
        };
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

        let verified = wallet
            .verify_utxos(vec![server_utxo])
            .expect("wallet should recognize its change output");

        assert_eq!(verified[0].keychain, WalletKeychain::Change);
        assert_eq!(verified[0].derivation_index, 0);
        assert_eq!(verified[0].amount.to_sat(), 899_998_360);
    }

    #[test]
    fn next_change_address_is_stable_until_reserved() {
        let wallet = UnlockedWallet {
            secrets: DecryptedWalletSecrets::new([0_u8; 16]),
            encryption_key: Zeroizing::new([7_u8; 32]),
            salt: [9_u8; 16],
        };

        let first = wallet
            .next_change_address()
            .expect("first change address should derive");
        let second = wallet
            .next_change_address()
            .expect("same change address should derive again");

        assert_eq!(first, second);
        assert!(p2qrh::script_pubkey_from_address(&first, crate::config::WALLET_NETWORK).is_ok());
    }

    #[test]
    fn receiving_address_allocation_stops_after_nineteen_unused_addresses() {
        let mut secrets = DecryptedWalletSecrets::new([0_u8; 16]);

        for _ in 0..19 {
            secrets
                .advance_receiving_index()
                .expect("test receiving address should be allocated");
        }

        let wallet = UnlockedWallet {
            secrets,
            encryption_key: Zeroizing::new([7_u8; 32]),
            salt: [9_u8; 16],
        };
        let checked_addresses = wallet
            .recent_receiving_addresses_for_allocation_check()
            .expect("recent receiving addresses should derive");

        assert_eq!(checked_addresses.len(), 19);
        assert_eq!(
            wallet.can_allocate_receiving_address(&checked_addresses, &[]),
            Ok(false)
        );
    }

    #[test]
    fn used_address_allows_another_receiving_address() {
        let mut secrets = DecryptedWalletSecrets::new([0_u8; 16]);

        for _ in 0..19 {
            secrets
                .advance_receiving_index()
                .expect("test receiving address should be allocated");
        }

        let wallet = UnlockedWallet {
            secrets,
            encryption_key: Zeroizing::new([7_u8; 32]),
            salt: [9_u8; 16],
        };
        let checked_addresses = wallet
            .recent_receiving_addresses_for_allocation_check()
            .expect("recent receiving addresses should derive");
        let used_addresses = vec![checked_addresses[0].clone()];

        assert_eq!(
            wallet.can_allocate_receiving_address(&checked_addresses, &used_addresses),
            Ok(true)
        );
    }

    #[test]
    fn wallet_password_can_be_changed() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let wallet_path = directory.path().join("wallet.json");
        let secrets = DecryptedWalletSecrets::new([42_u8; 16]);
        let encrypted_wallet = secrets
            .encrypt("current password")
            .expect("test wallet should encrypt");
        crate::wallet_file::save(&wallet_path, &encrypted_wallet).expect("test wallet should save");
        let mut wallet =
            UnlockedWallet::from_encrypted_wallet(&encrypted_wallet, "current password")
                .expect("test wallet should unlock");

        wallet
            .change_password_at_path(
                &wallet_path,
                "current password",
                "replacement password",
                "replacement password",
            )
            .expect("wallet password should change");

        let saved_wallet =
            crate::wallet_file::load(&wallet_path).expect("changed wallet should load");
        assert!(saved_wallet.decrypt_wallet("current password").is_err());
        assert!(saved_wallet.decrypt_wallet("replacement password").is_ok());
    }

    #[test]
    fn incorrect_current_password_does_not_replace_wallet_file() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let wallet_path = directory.path().join("wallet.json");
        let secrets = DecryptedWalletSecrets::new([42_u8; 16]);
        let encrypted_wallet = secrets
            .encrypt("current password")
            .expect("test wallet should encrypt");
        crate::wallet_file::save(&wallet_path, &encrypted_wallet).expect("test wallet should save");
        let mut wallet =
            UnlockedWallet::from_encrypted_wallet(&encrypted_wallet, "current password")
                .expect("test wallet should unlock");

        let result = wallet.change_password_at_path(
            &wallet_path,
            "incorrect password",
            "replacement password",
            "replacement password",
        );

        assert!(result.is_err());
        let saved_wallet =
            crate::wallet_file::load(&wallet_path).expect("original wallet should still load");
        assert!(saved_wallet.decrypt_wallet("current password").is_ok());
        assert!(saved_wallet.decrypt_wallet("replacement password").is_err());
    }
}
