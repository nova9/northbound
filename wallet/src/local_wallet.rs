use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use bitcoin::hex::FromHex;
use bitcoin::{Amount, OutPoint, ScriptBuf, Transaction, Txid};
use shared::WalletUtxo;

use crate::config::WALLET_NETWORK;
use crate::keys;
use crate::p2qrh;
use crate::wallet_file::{self, WalletFileVersion};
use crate::wallet_secrets::DecryptedWalletSecrets;

const ADDRESS_DISCOVERY_GAP_LIMIT: u32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivingAddress {
    pub index: u32,
    pub address: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredWalletAddresses {
    pub receiving_addresses: Vec<ReceivingAddress>,
    pub change_addresses: Vec<String>,
}

impl DiscoveredWalletAddresses {
    pub fn active_receiving_address(&self) -> Result<&ReceivingAddress, String> {
        self.receiving_addresses
            .last()
            .ok_or_else(|| "wallet has no receiving address".to_owned())
    }

    pub fn active_change_address(&self) -> Result<&str, String> {
        self.change_addresses
            .last()
            .map(String::as_str)
            .ok_or_else(|| "wallet has no change address".to_owned())
    }

    pub fn tracked_addresses(&self) -> Vec<String> {
        let mut addresses =
            Vec::with_capacity(self.receiving_addresses.len() + self.change_addresses.len());

        for receiving in &self.receiving_addresses {
            addresses.push(receiving.address.clone());
        }

        for change in &self.change_addresses {
            addresses.push(change.clone());
        }

        addresses
    }
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

#[derive(Clone, Debug)]
pub struct WalletAddressScanner {
    public_keys: keys::AccountPublicKeys,
}

impl WalletAddressScanner {
    pub fn from_entropy(entropy: &[u8; 16]) -> Result<Self, String> {
        let public_keys = keys::AccountPublicKeys::from_entropy(entropy)?;
        Ok(Self { public_keys })
    }

    pub async fn discover_addresses(&self) -> Result<DiscoveredWalletAddresses, String> {
        let receiving_addresses = self.discover_receiving_addresses().await?;
        let active_change_index = self
            .scan_keychain_for_active_index(WalletKeychain::Change)
            .await?;
        let change_addresses = self.derive_change_addresses_through(active_change_index)?;

        Ok(DiscoveredWalletAddresses {
            receiving_addresses,
            change_addresses,
        })
    }

    pub async fn discover_receiving_addresses(&self) -> Result<Vec<ReceivingAddress>, String> {
        let active_receiving_index = self
            .scan_keychain_for_active_index(WalletKeychain::Receiving)
            .await?;
        self.derive_receiving_addresses_through(active_receiving_index)
    }

    pub fn receiving_address_at(&self, index: u32) -> Result<String, String> {
        self.address_at(WalletKeychain::Receiving, index)
    }

    pub fn change_address_at(&self, index: u32) -> Result<String, String> {
        self.address_at(WalletKeychain::Change, index)
    }

    pub fn verify_utxos(
        &self,
        utxos: Vec<WalletUtxo>,
        discovered: &DiscoveredWalletAddresses,
    ) -> Result<Vec<VerifiedUtxo>, String> {
        let mut derivation_by_script = HashMap::new();

        for receiving in &discovered.receiving_addresses {
            let public_key = self.public_keys.receiving_public_key(receiving.index)?;
            let script_pubkey = p2qrh::script_pubkey_from_public_key(&public_key).to_vec();
            derivation_by_script
                .insert(script_pubkey, (WalletKeychain::Receiving, receiving.index));
        }

        for index in 0..discovered.change_addresses.len() {
            let index = u32::try_from(index)
                .map_err(|_| "wallet change address index is exhausted".to_owned())?;
            let public_key = self.public_keys.change_public_key(index)?;
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

    async fn scan_keychain_for_active_index(
        &self,
        keychain: WalletKeychain,
    ) -> Result<u32, String> {
        let url = crate::api::address_activity_endpoint()?;
        self.scan_keychain_for_active_index_from_server(keychain, &url)
            .await
    }

    async fn scan_keychain_for_active_index_from_server(
        &self,
        keychain: WalletKeychain,
        server_url: &str,
    ) -> Result<u32, String> {
        let mut batch_start = 0_u32;
        let mut last_used_index = None;

        loop {
            let mut addresses = Vec::with_capacity(ADDRESS_DISCOVERY_GAP_LIMIT as usize);

            for offset in 0..ADDRESS_DISCOVERY_GAP_LIMIT {
                let index = batch_start
                    .checked_add(offset)
                    .ok_or_else(|| "wallet address index is exhausted".to_owned())?;
                addresses.push(self.address_at(keychain, index)?);
            }

            let used_addresses =
                crate::api::fetch_used_addresses_from_server(addresses.clone(), server_url).await?;
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
                        .map_err(|_| "wallet address scan batch is too large".to_owned())?;
                    last_used_index = Some(
                        batch_start
                            .checked_add(offset)
                            .ok_or_else(|| "wallet address index is exhausted".to_owned())?,
                    );
                }
            }

            let batch_end = batch_start
                .checked_add(ADDRESS_DISCOVERY_GAP_LIMIT - 1)
                .ok_or_else(|| "wallet address index is exhausted".to_owned())?;
            let unused_after_last_used = match last_used_index {
                Some(index) => batch_end - index,
                None => batch_end + 1,
            };

            if unused_after_last_used >= ADDRESS_DISCOVERY_GAP_LIMIT {
                return match last_used_index {
                    Some(index) => index
                        .checked_add(1)
                        .ok_or_else(|| "wallet address index is exhausted".to_owned()),
                    None => Ok(0),
                };
            }

            batch_start = batch_start
                .checked_add(ADDRESS_DISCOVERY_GAP_LIMIT)
                .ok_or_else(|| "wallet address index is exhausted".to_owned())?;
        }
    }

    fn derive_receiving_addresses_through(
        &self,
        last_index: u32,
    ) -> Result<Vec<ReceivingAddress>, String> {
        let mut addresses = Vec::new();

        for index in 0..=last_index {
            let address = self.receiving_address_at(index)?;
            addresses.push(ReceivingAddress { index, address });
        }

        Ok(addresses)
    }

    fn derive_change_addresses_through(&self, last_index: u32) -> Result<Vec<String>, String> {
        let mut addresses = Vec::new();

        for index in 0..=last_index {
            addresses.push(self.change_address_at(index)?);
        }

        Ok(addresses)
    }

    fn address_at(&self, keychain: WalletKeychain, index: u32) -> Result<String, String> {
        let public_key = match keychain {
            WalletKeychain::Receiving => self.public_keys.receiving_public_key(index)?,
            WalletKeychain::Change => self.public_keys.change_public_key(index)?,
        };

        p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)
    }
}

pub struct UnlockedWallet {
    path: PathBuf,
    secrets: DecryptedWalletSecrets,
    address_scanner: WalletAddressScanner,
}

impl UnlockedWallet {
    fn new(path: &Path, secrets: DecryptedWalletSecrets) -> Result<Self, String> {
        let address_scanner = WalletAddressScanner::from_entropy(secrets.bip39_entropy())?;

        Ok(Self {
            path: path.to_path_buf(),
            secrets,
            address_scanner,
        })
    }

    pub fn address_scanner(&self) -> WalletAddressScanner {
        self.address_scanner.clone()
    }

    pub fn confirms_recovery_phrase(&self, recovery_phrase: &str) -> Result<bool, String> {
        keys::recovery_phrase_matches_entropy(recovery_phrase, self.secrets.bip39_entropy())
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
        let path = self.path.clone();
        self.change_password_at_path(
            &path,
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

        let loaded = wallet_file::load(wallet_path)?;
        let current_encryption_key = loaded
            .encrypted_wallet
            .derive_encryption_key(current_password)?;
        let saved_secrets = match loaded.version {
            WalletFileVersion::V1 => loaded
                .encrypted_wallet
                .decrypt_v1_wallet_with_key(&current_encryption_key)?,
            WalletFileVersion::V2 => loaded
                .encrypted_wallet
                .decrypt_wallet_with_key(&current_encryption_key)?,
        };

        if !saved_secrets.matches_entropy(self.secrets.bip39_entropy()) {
            return Err(
                "wallet file changed; unlock the wallet again before changing its password"
                    .to_owned(),
            );
        }

        let updated_encrypted_wallet = self.secrets.encrypt(new_password)?;
        wallet_file::save(wallet_path, &updated_encrypted_wallet)?;

        Ok(())
    }
}

pub fn save_new_wallet_and_unlock(
    wallet_path: &Path,
    pending_wallet: &keys::PendingWallet,
    entered_phrase: &str,
    password: &str,
    password_confirmation: &str,
) -> Result<UnlockedWallet, String> {
    let phrase_matches = pending_wallet.confirms_recovery_phrase(entered_phrase)?;

    if !phrase_matches {
        return Err("recovery phrase does not match".to_owned());
    }

    validate_new_password(password, password_confirmation)?;

    if wallet_path.exists() {
        return Err(format!(
            "a wallet already exists at {}",
            wallet_path.display()
        ));
    }

    let encrypted_wallet = pending_wallet.encrypt(password)?;
    let unlocked_wallet = UnlockedWallet::new(wallet_path, pending_wallet.secrets().clone())?;

    create_wallet_directory(wallet_path)?;
    wallet_file::save_new(wallet_path, &encrypted_wallet)?;

    Ok(unlocked_wallet)
}

pub fn save_restored_wallet_and_unlock(
    wallet_path: &Path,
    recovery_phrase: &str,
    password: &str,
    password_confirmation: &str,
) -> Result<UnlockedWallet, String> {
    validate_new_password(password, password_confirmation)?;

    let restored_wallet = keys::restore_wallet(recovery_phrase)?;

    if wallet_path.exists() {
        return Err(format!(
            "a wallet already exists at {}",
            wallet_path.display()
        ));
    }

    let encrypted_wallet = restored_wallet.encrypt(password)?;
    let unlocked_wallet = UnlockedWallet::new(wallet_path, restored_wallet.secrets().clone())?;

    create_wallet_directory(wallet_path)?;
    wallet_file::save_new(wallet_path, &encrypted_wallet)?;

    Ok(unlocked_wallet)
}

pub fn load_and_unlock_wallet(
    wallet_path: &Path,
    password: &str,
) -> Result<UnlockedWallet, String> {
    let loaded = wallet_file::load(wallet_path)?;
    let encryption_key = loaded.encrypted_wallet.derive_encryption_key(password)?;

    let secrets = match loaded.version {
        WalletFileVersion::V1 => {
            let secrets = loaded
                .encrypted_wallet
                .decrypt_v1_wallet_with_key(&encryption_key)?;
            // Upgrade version 1 wallet files in place once decrypted so legacy
            // address-index counters are removed from disk.
            let upgraded_wallet = secrets.encrypt_with_key(
                loaded.encrypted_wallet.salt(),
                &encryption_key,
                loaded.encrypted_wallet.key_derivation_parameters(),
            )?;
            wallet_file::save(wallet_path, &upgraded_wallet)?;
            secrets
        }
        WalletFileVersion::V2 => loaded
            .encrypted_wallet
            .decrypt_wallet_with_key(&encryption_key)?,
    };

    UnlockedWallet::new(wallet_path, secrets)
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

#[cfg(test)]
mod tests {
    use shared::WalletUtxo;

    use super::{
        DiscoveredWalletAddresses, ReceivingAddress, UnlockedWallet, WalletAddressScanner,
        WalletKeychain,
    };
    use crate::keys;
    use crate::p2qrh;
    use crate::wallet_file::{self, WalletFileVersion};
    use crate::wallet_secrets::{
        CURRENT_KEY_DERIVATION_PARAMETERS, DecryptedWalletSecrets, EncryptedWalletSecrets,
        KeyDerivationParameters,
    };

    fn write_version_one_wallet_file(
        wallet_path: &std::path::Path,
        entropy: [u8; 16],
        password: &str,
        next_receiving_index: u32,
        next_change_index: u32,
    ) {
        let payload = serde_json::json!({
            "bip39_entropy": entropy,
            "next_receiving_index": next_receiving_index,
            "next_change_index": next_change_index,
        });
        let encrypted = EncryptedWalletSecrets::encrypt(
            &serde_json::to_vec(&payload).expect("legacy payload should serialize"),
            password,
        )
        .expect("legacy wallet should encrypt");
        wallet_file::save(wallet_path, &encrypted).expect("temporary wallet should save");
        let v1_json = std::fs::read_to_string(wallet_path)
            .expect("saved wallet should be readable")
            .replace(r#""version": 2"#, r#""version": 1"#);
        std::fs::write(wallet_path, v1_json).expect("v1 wallet file should write");
    }

    #[test]
    fn unlocking_version_one_wallet_upgrades_file_to_version_two() {
        let directory = tempfile::tempdir().expect("temporary wallet folder");
        let wallet_path = directory.path().join("Legacy.json");
        let pending = keys::generate_new_wallet().expect("wallet should generate");
        let entropy = *pending.secrets().bip39_entropy();

        write_version_one_wallet_file(&wallet_path, entropy, "legacy password", 9, 4);
        assert_eq!(
            wallet_file::load(&wallet_path)
                .expect("v1 file should load")
                .version,
            WalletFileVersion::V1
        );

        let unlocked = super::load_and_unlock_wallet(&wallet_path, "legacy password")
            .expect("v1 wallet should unlock and upgrade");
        assert!(
            unlocked
                .confirms_recovery_phrase(pending.recovery_phrase())
                .expect("phrase check should succeed")
        );

        let upgraded_contents =
            std::fs::read_to_string(&wallet_path).expect("upgraded wallet should be readable");
        assert!(upgraded_contents.contains(r#""version": 2"#));

        let loaded = wallet_file::load(&wallet_path).expect("upgraded file should load");
        assert_eq!(loaded.version, WalletFileVersion::V2);

        let reopened = super::load_and_unlock_wallet(&wallet_path, "legacy password")
            .expect("upgraded v2 wallet should unlock");
        assert!(
            reopened
                .confirms_recovery_phrase(pending.recovery_phrase())
                .expect("phrase check should succeed")
        );
    }

    #[test]
    fn wrong_password_leaves_version_one_wallet_file_untouched() {
        let directory = tempfile::tempdir().expect("temporary wallet folder");
        let wallet_path = directory.path().join("Legacy.json");

        write_version_one_wallet_file(&wallet_path, [42_u8; 16], "legacy password", 3, 1);
        let original = std::fs::read(&wallet_path).expect("v1 wallet should be readable");

        assert!(super::load_and_unlock_wallet(&wallet_path, "wrong password").is_err());
        assert_eq!(
            std::fs::read(&wallet_path).expect("wallet should remain readable"),
            original
        );
    }

    #[test]
    fn mining_rewards_enter_spendable_balance_at_one_hundred_confirmations() {
        let mut reward = super::VerifiedUtxo {
            outpoint: bitcoin::OutPoint::null(),
            amount: bitcoin::Amount::from_sat(51_200_000_000),
            script_pubkey: bitcoin::ScriptBuf::new(),
            keychain: WalletKeychain::Receiving,
            derivation_index: 0,
            confirmations: 68,
            coinbase: true,
        };
        assert_eq!(
            super::calculate_spendable_balance(&[reward.clone()]).unwrap(),
            0
        );
        reward.confirmations = 99;
        assert!(!reward.is_spendable());
        reward.confirmations = 100;
        assert_eq!(
            super::calculate_spendable_balance(&[reward]).unwrap(),
            51_200_000_000
        );
    }

    #[test]
    fn wallets_keep_their_passwords_in_separate_files() {
        let directory = tempfile::tempdir().expect("temporary wallet folder");
        let first_path = directory.path().join("Spending.json");
        let second_path = directory.path().join("Savings.json");
        let first_pending = keys::generate_new_wallet().expect("first wallet");
        let second_pending = keys::generate_new_wallet().expect("second wallet");
        let mut first = super::save_new_wallet_and_unlock(
            &first_path,
            &first_pending,
            first_pending.recovery_phrase(),
            "first wallet password",
            "first wallet password",
        )
        .expect("save first wallet");
        let _second = super::save_new_wallet_and_unlock(
            &second_path,
            &second_pending,
            second_pending.recovery_phrase(),
            "second wallet password",
            "second wallet password",
        )
        .expect("save second wallet");
        let second_before = std::fs::read(&second_path).expect("read second wallet");

        first
            .change_password(
                "first wallet password",
                "updated first password",
                "updated first password",
            )
            .expect("change first password");
        assert_eq!(std::fs::read(&second_path).unwrap(), second_before);

        assert!(super::load_and_unlock_wallet(&first_path, "first wallet password").is_err());
        assert!(super::load_and_unlock_wallet(&second_path, "updated first password").is_err());
        let first = super::load_and_unlock_wallet(&first_path, "updated first password").unwrap();
        let second = super::load_and_unlock_wallet(&second_path, "second wallet password").unwrap();
        assert!(
            first
                .confirms_recovery_phrase(first_pending.recovery_phrase())
                .unwrap()
        );
        assert!(
            second
                .confirms_recovery_phrase(second_pending.recovery_phrase())
                .unwrap()
        );
    }

    #[test]
    fn restoring_another_wallet_saves_to_its_own_version_two_file() {
        let directory = tempfile::tempdir().expect("temporary wallet folder");
        let existing_path = directory.path().join("Existing.json");
        let restored_path = directory.path().join("Restored.json");
        std::fs::write(&existing_path, "leave this wallet untouched").unwrap();
        let pending = keys::generate_new_wallet().expect("wallet phrase");

        let restored = super::save_restored_wallet_and_unlock(
            &restored_path,
            pending.recovery_phrase(),
            "restored password",
            "restored password",
        )
        .expect("restore wallet");
        assert!(
            restored
                .confirms_recovery_phrase(pending.recovery_phrase())
                .unwrap()
        );

        let loaded = wallet_file::load(&restored_path).unwrap();
        assert_eq!(loaded.version, WalletFileVersion::V2);
        let reopened = super::load_and_unlock_wallet(&restored_path, "restored password").unwrap();
        assert!(
            reopened
                .confirms_recovery_phrase(pending.recovery_phrase())
                .unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(existing_path).unwrap(),
            "leave this wallet untouched"
        );
    }

    #[test]
    fn restoration_cannot_overwrite_an_existing_wallet_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Restored.json");
        let pending = keys::generate_new_wallet().unwrap();
        std::fs::write(&path, "another wallet was created here").unwrap();

        assert!(
            super::save_restored_wallet_and_unlock(
                &path,
                pending.recovery_phrase(),
                "restored password",
                "restored password",
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "another wallet was created here"
        );
    }

    #[test]
    fn unlocked_wallet_confirms_its_recovery_phrase() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let wallet_path = directory.path().join("Test.json");
        let pending_wallet = keys::generate_new_wallet().expect("wallet should be generated");
        let recovery_phrase = pending_wallet.recovery_phrase().to_owned();
        let unlocked_wallet = UnlockedWallet::new(&wallet_path, pending_wallet.secrets().clone())
            .expect("wallet should unlock");

        let phrase_matches = unlocked_wallet
            .confirms_recovery_phrase(&recovery_phrase)
            .expect("recovery phrase should be valid");

        assert!(phrase_matches);
    }

    #[test]
    fn unlocked_wallet_rejects_another_recovery_phrase() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let wallet_path = directory.path().join("Test.json");
        let pending_wallet = keys::generate_new_wallet().expect("wallet should be generated");
        let other_wallet = keys::generate_new_wallet().expect("second wallet should be generated");
        let other_recovery_phrase = other_wallet.recovery_phrase().to_owned();
        let unlocked_wallet = UnlockedWallet::new(&wallet_path, pending_wallet.secrets().clone())
            .expect("wallet should unlock");

        let phrase_matches = unlocked_wallet
            .confirms_recovery_phrase(&other_recovery_phrase)
            .expect("recovery phrase should be valid");

        assert!(!phrase_matches);
    }

    #[test]
    fn utxo_is_mapped_to_its_receiving_derivation_index() {
        let scanner = WalletAddressScanner::from_entropy(&[0_u8; 16]).expect("scanner");
        let discovered = DiscoveredWalletAddresses {
            receiving_addresses: vec![ReceivingAddress {
                index: 0,
                address: scanner.receiving_address_at(0).expect("address 0"),
            }],
            change_addresses: vec![scanner.change_address_at(0).expect("change 0")],
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

        let verified = scanner
            .verify_utxos(vec![server_utxo], &discovered)
            .expect("wallet should recognize its output");

        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].keychain, WalletKeychain::Receiving);
        assert_eq!(verified[0].derivation_index, 0);
        assert_eq!(verified[0].amount.to_sat(), 1_000_000_000);
    }

    #[test]
    fn utxo_is_mapped_to_its_change_derivation_index() {
        let scanner = WalletAddressScanner::from_entropy(&[0_u8; 16]).expect("scanner");
        let discovered = DiscoveredWalletAddresses {
            receiving_addresses: vec![ReceivingAddress {
                index: 0,
                address: scanner.receiving_address_at(0).expect("address 0"),
            }],
            change_addresses: vec![scanner.change_address_at(0).expect("change 0")],
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

        let verified = scanner
            .verify_utxos(vec![server_utxo], &discovered)
            .expect("wallet should recognize its change output");

        assert_eq!(verified[0].keychain, WalletKeychain::Change);
        assert_eq!(verified[0].derivation_index, 0);
        assert_eq!(verified[0].amount.to_sat(), 899_998_360);
    }

    #[test]
    fn wallet_password_can_be_changed() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let wallet_path = directory.path().join("wallet.json");
        let secrets = DecryptedWalletSecrets::new([42_u8; 16]);
        let encrypted_wallet = secrets
            .encrypt("current password")
            .expect("test wallet should encrypt");
        wallet_file::save(&wallet_path, &encrypted_wallet).expect("test wallet should save");
        let mut wallet = super::load_and_unlock_wallet(&wallet_path, "current password")
            .expect("test wallet should unlock");

        wallet
            .change_password_at_path(
                &wallet_path,
                "current password",
                "replacement password",
                "replacement password",
            )
            .expect("wallet password should change");

        let saved_wallet = wallet_file::load(&wallet_path).expect("changed wallet should load");
        assert!(
            saved_wallet
                .encrypted_wallet
                .decrypt_wallet("current password")
                .is_err()
        );
        assert!(
            saved_wallet
                .encrypted_wallet
                .decrypt_wallet("replacement password")
                .is_ok()
        );
    }

    #[test]
    fn changing_password_upgrades_legacy_key_derivation_parameters() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let wallet_path = directory.path().join("wallet.json");
        let secrets = DecryptedWalletSecrets::new([42_u8; 16]);
        let legacy_parameters = KeyDerivationParameters::new(19 * 1024, 2, 1)
            .expect("legacy parameters should remain supported");
        let encrypted_wallet = secrets
            .encrypt_with_parameters("current password", legacy_parameters)
            .expect("legacy wallet should encrypt");
        wallet_file::save(&wallet_path, &encrypted_wallet).expect("legacy wallet should save");
        let mut wallet = super::load_and_unlock_wallet(&wallet_path, "current password")
            .expect("legacy wallet should unlock");

        wallet
            .change_password_at_path(
                &wallet_path,
                "current password",
                "replacement password",
                "replacement password",
            )
            .expect("wallet password should change");

        let saved_wallet = wallet_file::load(&wallet_path).expect("upgraded wallet should load");
        assert_eq!(
            saved_wallet.encrypted_wallet.key_derivation_parameters(),
            CURRENT_KEY_DERIVATION_PARAMETERS
        );
        assert!(
            saved_wallet
                .encrypted_wallet
                .decrypt_wallet("replacement password")
                .is_ok()
        );
    }

    #[test]
    fn incorrect_current_password_does_not_replace_wallet_file() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let wallet_path = directory.path().join("wallet.json");
        let secrets = DecryptedWalletSecrets::new([42_u8; 16]);
        let encrypted_wallet = secrets
            .encrypt("current password")
            .expect("test wallet should encrypt");
        wallet_file::save(&wallet_path, &encrypted_wallet).expect("test wallet should save");
        let mut wallet = super::load_and_unlock_wallet(&wallet_path, "current password")
            .expect("test wallet should unlock");

        let result = wallet.change_password_at_path(
            &wallet_path,
            "incorrect password",
            "replacement password",
            "replacement password",
        );

        assert!(result.is_err());
        let saved_wallet =
            wallet_file::load(&wallet_path).expect("original wallet should still load");
        assert!(
            saved_wallet
                .encrypted_wallet
                .decrypt_wallet("current password")
                .is_ok()
        );
        assert!(
            saved_wallet
                .encrypted_wallet
                .decrypt_wallet("replacement password")
                .is_err()
        );
    }

    #[test]
    fn address_discovery_finds_first_unused_index_across_gap_batches() {
        let scanner = WalletAddressScanner::from_entropy(&[0_u8; 16]).expect("scanner");
        let addr_0 = scanner.receiving_address_at(0).unwrap();
        let addr_19 = scanner.receiving_address_at(19).unwrap();
        let addr_20 = scanner.receiving_address_at(20).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        // Unused wallet: returns active index 0 in a single batch.
        let (url, server) = crate::api::test_support::serve(1, |_, _| {
            (
                200,
                serde_json::json!({
                    "used_addresses": [],
                    "index": crate::api::test_support::index(),
                }),
            )
        });
        let index = runtime
            .block_on(
                scanner.scan_keychain_for_active_index_from_server(WalletKeychain::Receiving, &url),
            )
            .unwrap();
        assert_eq!(index, 0);
        server.join().unwrap();

        // Address 0 and 19 used: scans batches 0..20 and 20..40, stops with active index 20.
        let used_set = vec![addr_0.clone(), addr_19.clone()];
        let (url, server) = crate::api::test_support::serve(2, move |_, request| {
            let used = request["addresses"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|value| value.as_str())
                .filter(|address| used_set.iter().any(|used| used == address))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            (
                200,
                serde_json::json!({
                    "used_addresses": used,
                    "index": crate::api::test_support::index(),
                }),
            )
        });
        let index = runtime
            .block_on(
                scanner.scan_keychain_for_active_index_from_server(WalletKeychain::Receiving, &url),
            )
            .unwrap();
        assert_eq!(index, 20);
        server.join().unwrap();

        // Address 19 and 20 used: scans 0..20, 20..40, and 40..60, stops with active index 21.
        let used_set = vec![addr_19, addr_20];
        let (url, server) = crate::api::test_support::serve(3, move |_, request| {
            let used = request["addresses"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|value| value.as_str())
                .filter(|address| used_set.iter().any(|used| used == address))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            (
                200,
                serde_json::json!({
                    "used_addresses": used,
                    "index": crate::api::test_support::index(),
                }),
            )
        });
        let index = runtime
            .block_on(
                scanner.scan_keychain_for_active_index_from_server(WalletKeychain::Receiving, &url),
            )
            .unwrap();
        assert_eq!(index, 21);
        server.join().unwrap();
    }
}
