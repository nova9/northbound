use std::path::{Path, PathBuf};

use zeroize::Zeroize;

use crate::crypto::keys;
use crate::server::WalletAddressScanner;
use crate::storage::secrets::{DecryptedWalletSecrets, EncryptedWalletSecrets};
use crate::storage::wallet_file::{self, WalletFileVersion};
use crate::transactions::{self, SignedTransaction, TransactionPlan};

pub struct PendingWallet {
    recovery_phrase: String,
    secrets: DecryptedWalletSecrets,
}

impl PendingWallet {
    pub fn recovery_phrase(&self) -> &str {
        &self.recovery_phrase
    }

    pub fn confirms_recovery_phrase(&self, entered_phrase: &str) -> Result<bool, String> {
        keys::recovery_phrase_matches_entropy(entered_phrase, self.secrets.bip39_entropy())
    }

    fn encrypt(&self, password: &str) -> Result<EncryptedWalletSecrets, String> {
        self.secrets.encrypt(password)
    }

    fn secrets(&self) -> &DecryptedWalletSecrets {
        &self.secrets
    }
}

impl Drop for PendingWallet {
    fn drop(&mut self) {
        self.recovery_phrase.zeroize();
    }
}

pub fn generate_new_wallet() -> Result<PendingWallet, String> {
    let (recovery_phrase, entropy) = keys::generate_recovery_phrase_and_entropy()?;
    Ok(PendingWallet {
        recovery_phrase,
        secrets: DecryptedWalletSecrets::new(entropy),
    })
}

pub fn restore_wallet(recovery_phrase: &str) -> Result<PendingWallet, String> {
    let (recovery_phrase, entropy) = keys::parse_recovery_phrase_and_entropy(recovery_phrase)?;
    Ok(PendingWallet {
        recovery_phrase,
        secrets: DecryptedWalletSecrets::new(entropy),
    })
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

    pub fn build_and_sign_transaction(
        &self,
        plan: &TransactionPlan,
        recipient_address: &str,
    ) -> Result<SignedTransaction, String> {
        transactions::build_and_sign_p2qrh_transaction(
            plan,
            recipient_address,
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
    pending_wallet: &PendingWallet,
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

    let restored_wallet = restore_wallet(recovery_phrase)?;

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
#[path = "wallet_tests.rs"]
mod tests;
