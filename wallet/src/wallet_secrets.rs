use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

pub(crate) const KEY_DERIVATION_ALGORITHM: &str = "argon2id";
pub(crate) const ENCRYPTION_ALGORITHM: &str = "xchacha20poly1305";

const ENCRYPTION_KEY_LENGTH: usize = 32;
pub(crate) type EncryptionKey = Zeroizing<[u8; ENCRYPTION_KEY_LENGTH]>;

pub(crate) const ARGON2_MEMORY_COST_KIB: u32 = 19 * 1024;
pub(crate) const ARGON2_ITERATIONS: u32 = 2;
pub(crate) const ARGON2_PARALLELISM: u32 = 1;

pub const SALT_LENGTH: usize = 16;
pub const NONCE_LENGTH: usize = 24;

#[derive(Serialize, Deserialize, Clone)]
pub struct DecryptedWalletSecrets {
    bip39_entropy: [u8; 16],
    next_receiving_index: u32,
    next_change_index: u32,
}

impl Drop for DecryptedWalletSecrets {
    fn drop(&mut self) {
        self.bip39_entropy.zeroize();
    }
}

impl DecryptedWalletSecrets {
    pub fn new(bip39_entropy: [u8; 16]) -> Self {
        Self {
            bip39_entropy,
            next_receiving_index: 0,
            next_change_index: 0,
        }
    }

    pub fn encrypt(&self, password: &str) -> Result<EncryptedWalletSecrets, String> {
        let plaintext = self.serialize_for_encryption()?;

        EncryptedWalletSecrets::encrypt(&plaintext, password)
    }

    pub(crate) fn encrypt_with_key(
        &self,
        salt: &[u8; SALT_LENGTH],
        encryption_key: &EncryptionKey,
    ) -> Result<EncryptedWalletSecrets, String> {
        let plaintext = self.serialize_for_encryption()?;

        EncryptedWalletSecrets::encrypt_with_key(&plaintext, salt, encryption_key)
    }

    fn serialize_for_encryption(&self) -> Result<Zeroizing<Vec<u8>>, String> {
        let plaintext = serde_json::to_vec(self)
            .map_err(|error| format!("could not encode wallet secrets: {error}"))?;

        Ok(Zeroizing::new(plaintext))
    }

    pub fn matches_entropy(&self, entropy: &[u8; 16]) -> bool {
        self.bip39_entropy == *entropy
    }

    pub(crate) fn bip39_entropy(&self) -> &[u8; 16] {
        &self.bip39_entropy
    }

    pub(crate) fn next_receiving_index(&self) -> u32 {
        self.next_receiving_index
    }

    pub(crate) fn advance_receiving_index(&mut self) -> Result<(), String> {
        self.next_receiving_index = self
            .next_receiving_index
            .checked_add(1)
            .ok_or_else(|| "receiving address index is exhausted".to_owned())?;

        Ok(())
    }

    pub(crate) fn next_change_index(&self) -> u32 {
        self.next_change_index
    }

    pub(crate) fn advance_change_index(&mut self) -> Result<(), String> {
        self.next_change_index = self
            .next_change_index
            .checked_add(1)
            .ok_or_else(|| "change address index is exhausted".to_owned())?;

        Ok(())
    }

    pub(crate) fn set_next_address_indexes(
        &mut self,
        next_receiving_index: u32,
        next_change_index: u32,
    ) {
        self.next_receiving_index = next_receiving_index;
        self.next_change_index = next_change_index;
    }
}

pub struct EncryptedWalletSecrets {
    salt: [u8; SALT_LENGTH],
    nonce: [u8; NONCE_LENGTH],
    ciphertext: Vec<u8>,
}

impl EncryptedWalletSecrets {
    pub fn encrypt(plaintext: &[u8], password: &str) -> Result<Self, String> {
        let mut salt = [0_u8; SALT_LENGTH];
        getrandom::fill(&mut salt)
            .map_err(|error| format!("could not generate wallet salt: {error}"))?;

        let encryption_key = derive_encryption_key(password, &salt)?;

        Self::encrypt_with_key(plaintext, &salt, &encryption_key)
    }

    #[cfg(test)]
    pub fn decrypt(&self, password: &str) -> Result<Vec<u8>, String> {
        decrypt_wallet_secrets(&self.ciphertext, password, &self.salt, &self.nonce)
    }

    #[cfg(test)]
    pub fn decrypt_wallet(&self, password: &str) -> Result<DecryptedWalletSecrets, String> {
        let encryption_key = self.derive_encryption_key(password)?;

        self.decrypt_wallet_with_key(&encryption_key)
    }

    pub(crate) fn derive_encryption_key(&self, password: &str) -> Result<EncryptionKey, String> {
        derive_encryption_key(password, &self.salt)
    }

    pub(crate) fn decrypt_wallet_with_key(
        &self,
        encryption_key: &EncryptionKey,
    ) -> Result<DecryptedWalletSecrets, String> {
        let mut plaintext =
            decrypt_wallet_secrets_with_key(&self.ciphertext, encryption_key, &self.nonce)?;

        let result = serde_json::from_slice(&plaintext)
            .map_err(|error| format!("could not decode wallet secrets: {error}"));

        plaintext.zeroize();

        result
    }

    fn encrypt_with_key(
        plaintext: &[u8],
        salt: &[u8; SALT_LENGTH],
        encryption_key: &EncryptionKey,
    ) -> Result<Self, String> {
        let mut nonce = [0_u8; NONCE_LENGTH];
        getrandom::fill(&mut nonce)
            .map_err(|error| format!("could not generate wallet nonce: {error}"))?;

        let ciphertext = encrypt_wallet_secrets_with_key(plaintext, encryption_key, &nonce)?;

        Ok(Self {
            salt: *salt,
            nonce,
            ciphertext,
        })
    }

    pub(crate) fn salt(&self) -> &[u8; SALT_LENGTH] {
        &self.salt
    }

    pub(crate) fn nonce(&self) -> &[u8; NONCE_LENGTH] {
        &self.nonce
    }

    pub(crate) fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }

    pub(crate) fn from_encrypted_parts(
        salt: [u8; SALT_LENGTH],
        nonce: [u8; NONCE_LENGTH],
        ciphertext: Vec<u8>,
    ) -> Result<Self, String> {
        if ciphertext.len() < 16 {
            return Err("wallet ciphertext is too short".to_owned());
        }

        Ok(Self {
            salt,
            nonce,
            ciphertext,
        })
    }
}

#[cfg(test)]
pub fn encrypt_wallet_secrets(
    plaintext: &[u8],
    password: &str,
    salt: &[u8; SALT_LENGTH],
    nonce: &[u8; NONCE_LENGTH],
) -> Result<Vec<u8>, String> {
    let encryption_key = derive_encryption_key(password, salt)?;

    encrypt_wallet_secrets_with_key(plaintext, &encryption_key, nonce)
}

fn encrypt_wallet_secrets_with_key(
    plaintext: &[u8],
    encryption_key: &EncryptionKey,
    nonce: &[u8; NONCE_LENGTH],
) -> Result<Vec<u8>, String> {
    let cipher = XChaCha20Poly1305::new_from_slice(encryption_key.as_ref())
        .map_err(|_| "could not create wallet cipher".to_owned())?;

    let nonce = XNonce::try_from(nonce.as_slice())
        .map_err(|_| "wallet nonce has an invalid length".to_owned())?;

    let result = cipher.encrypt(&nonce, plaintext);

    result.map_err(|_| "could not encrypt wallet secrets".to_owned())
}

#[cfg(test)]
pub fn decrypt_wallet_secrets(
    ciphertext: &[u8],
    password: &str,
    salt: &[u8; SALT_LENGTH],
    nonce: &[u8; NONCE_LENGTH],
) -> Result<Vec<u8>, String> {
    let encryption_key = derive_encryption_key(password, salt)?;

    decrypt_wallet_secrets_with_key(ciphertext, &encryption_key, nonce)
}

fn decrypt_wallet_secrets_with_key(
    ciphertext: &[u8],
    encryption_key: &EncryptionKey,
    nonce: &[u8; NONCE_LENGTH],
) -> Result<Vec<u8>, String> {
    let cipher = XChaCha20Poly1305::new_from_slice(encryption_key.as_ref())
        .map_err(|_| "could not create wallet cipher".to_owned())?;

    let nonce = XNonce::try_from(nonce.as_slice())
        .map_err(|_| "wallet nonce has an invalid length".to_owned())?;

    let result = cipher.decrypt(&nonce, ciphertext);

    result.map_err(|_| "Incorrect password or damaged wallet file".to_owned())
}

fn derive_encryption_key(
    password: &str,
    salt: &[u8; SALT_LENGTH],
) -> Result<EncryptionKey, String> {
    let parameters = Params::new(
        ARGON2_MEMORY_COST_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        Some(ENCRYPTION_KEY_LENGTH),
    )
    .map_err(|error| format!("invalid wallet encryption parameters: {error}"))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, parameters);

    let mut encryption_key = Zeroizing::new([0_u8; ENCRYPTION_KEY_LENGTH]);

    argon2
        .hash_password_into(password.as_bytes(), salt, encryption_key.as_mut())
        .map_err(|error| format!("could not derive wallet encryption key: {error}"))?;

    Ok(encryption_key)
}

#[cfg(test)]
mod tests {
    use super::{
        DecryptedWalletSecrets, EncryptedWalletSecrets, decrypt_wallet_secrets,
        encrypt_wallet_secrets,
    };

    const TEST_SALT: [u8; 16] = [7; 16];
    const TEST_NONCE: [u8; 24] = [9; 24];

    #[test]
    fn correct_password_recovers_wallet_secrets() {
        let plaintext = b"example wallet secrets";

        let ciphertext =
            encrypt_wallet_secrets(plaintext, "correct horse", &TEST_SALT, &TEST_NONCE)
                .expect("test secrets should encrypt");

        let decrypted =
            decrypt_wallet_secrets(&ciphertext, "correct horse", &TEST_SALT, &TEST_NONCE)
                .expect("correct password should decrypt");

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let ciphertext = encrypt_wallet_secrets(
            b"example wallet secrets",
            "correct horse",
            &TEST_SALT,
            &TEST_NONCE,
        )
        .expect("test secrets should encrypt");

        let result = decrypt_wallet_secrets(&ciphertext, "wrong password", &TEST_SALT, &TEST_NONCE);

        assert!(result.is_err());
    }

    #[test]
    fn modified_ciphertext_is_rejected() {
        let mut ciphertext = encrypt_wallet_secrets(
            b"example wallet secrets",
            "correct horse",
            &TEST_SALT,
            &TEST_NONCE,
        )
        .expect("test secrets should encrypt");

        ciphertext[0] ^= 1;

        let result = decrypt_wallet_secrets(&ciphertext, "correct horse", &TEST_SALT, &TEST_NONCE);

        assert!(result.is_err());
    }

    #[test]
    fn generated_salt_and_nonce_produce_a_decryptable_bundle() {
        let encrypted = EncryptedWalletSecrets::encrypt(b"example wallet secrets", "correct horse")
            .expect("test secrets should encrypt");

        let decrypted = encrypted
            .decrypt("correct horse")
            .expect("correct password should decrypt");

        assert_eq!(decrypted, b"example wallet secrets");
    }

    #[test]
    fn retained_key_resaves_secrets_with_a_fresh_nonce() {
        let secrets = DecryptedWalletSecrets::new([42; 16]);
        let encrypted = secrets
            .encrypt("correct horse")
            .expect("wallet should encrypt");
        let encryption_key = encrypted
            .derive_encryption_key("correct horse")
            .expect("wallet key should derive");

        let resaved = secrets
            .encrypt_with_key(encrypted.salt(), &encryption_key)
            .expect("wallet should encrypt with the retained key");
        let restored = resaved
            .decrypt_wallet_with_key(&encryption_key)
            .expect("retained key should decrypt the resaved wallet");

        assert_eq!(resaved.salt, encrypted.salt);
        assert_ne!(resaved.nonce, encrypted.nonce);
        assert!(restored.matches_entropy(&[42; 16]));
    }

    #[test]
    fn wallet_payload_survives_encryption_roundtrip() {
        let original = DecryptedWalletSecrets {
            bip39_entropy: [42; 16],
            next_receiving_index: 3,
            next_change_index: 1,
        };

        let encrypted = original
            .encrypt("correct horse")
            .expect("wallet should encrypt");

        let restored = encrypted
            .decrypt_wallet("correct horse")
            .expect("wallet should decrypt");

        assert_eq!(restored.bip39_entropy, original.bip39_entropy);
        assert_eq!(restored.next_receiving_index, original.next_receiving_index);
        assert_eq!(restored.next_change_index, original.next_change_index);
    }
}
