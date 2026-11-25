use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

pub(crate) const KEY_DERIVATION_ALGORITHM: &str = "argon2id";
pub(crate) const ENCRYPTION_ALGORITHM: &str = "xchacha20poly1305";

const ENCRYPTION_KEY_LENGTH: usize = 32;
pub(crate) type EncryptionKey = Zeroizing<[u8; ENCRYPTION_KEY_LENGTH]>;

const MIN_ARGON2_MEMORY_COST_KIB: u32 = 19 * 1024;
const MAX_ARGON2_MEMORY_COST_KIB: u32 = 256 * 1024;
const MIN_ARGON2_ITERATIONS: u32 = 2;
const MAX_ARGON2_ITERATIONS: u32 = 10;
const MIN_ARGON2_PARALLELISM: u32 = 1;
const MAX_ARGON2_PARALLELISM: u32 = 16;

pub(crate) const CURRENT_KEY_DERIVATION_PARAMETERS: KeyDerivationParameters =
    KeyDerivationParameters {
        memory_cost_kib: 64 * 1024,
        iterations: 3,
        parallelism: 4,
    };

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KeyDerivationParameters {
    memory_cost_kib: u32,
    iterations: u32,
    parallelism: u32,
}

impl KeyDerivationParameters {
    pub(crate) fn new(
        memory_cost_kib: u32,
        iterations: u32,
        parallelism: u32,
    ) -> Result<Self, String> {
        if !(MIN_ARGON2_MEMORY_COST_KIB..=MAX_ARGON2_MEMORY_COST_KIB).contains(&memory_cost_kib) {
            return Err("wallet Argon2 memory cost is outside the supported range".to_owned());
        }

        if !(MIN_ARGON2_ITERATIONS..=MAX_ARGON2_ITERATIONS).contains(&iterations) {
            return Err("wallet Argon2 iteration count is outside the supported range".to_owned());
        }

        if !(MIN_ARGON2_PARALLELISM..=MAX_ARGON2_PARALLELISM).contains(&parallelism) {
            return Err("wallet Argon2 parallelism is outside the supported range".to_owned());
        }

        Ok(Self {
            memory_cost_kib,
            iterations,
            parallelism,
        })
    }

    pub(crate) fn memory_cost_kib(self) -> u32 {
        self.memory_cost_kib
    }

    pub(crate) fn iterations(self) -> u32 {
        self.iterations
    }

    pub(crate) fn parallelism(self) -> u32 {
        self.parallelism
    }
}

pub const SALT_LENGTH: usize = 16;
pub const NONCE_LENGTH: usize = 24;

#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DecryptedWalletSecrets {
    bip39_entropy: [u8; 16],
}

impl Drop for DecryptedWalletSecrets {
    fn drop(&mut self) {
        self.bip39_entropy.zeroize();
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWalletSecretsV1 {
    bip39_entropy: [u8; 16],
    #[allow(dead_code)]
    next_receiving_index: u32,
    #[allow(dead_code)]
    next_change_index: u32,
}

impl Drop for LegacyWalletSecretsV1 {
    fn drop(&mut self) {
        self.bip39_entropy.zeroize();
    }
}

impl DecryptedWalletSecrets {
    pub fn new(bip39_entropy: [u8; 16]) -> Self {
        Self { bip39_entropy }
    }

    pub fn encrypt(&self, password: &str) -> Result<EncryptedWalletSecrets, String> {
        let plaintext = self.serialize_for_encryption()?;

        EncryptedWalletSecrets::encrypt(&plaintext, password)
    }

    pub(crate) fn encrypt_with_key(
        &self,
        salt: &[u8; SALT_LENGTH],
        encryption_key: &EncryptionKey,
        key_derivation_parameters: KeyDerivationParameters,
    ) -> Result<EncryptedWalletSecrets, String> {
        let plaintext = self.serialize_for_encryption()?;

        EncryptedWalletSecrets::encrypt_with_key(
            &plaintext,
            salt,
            encryption_key,
            key_derivation_parameters,
        )
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
}

pub struct EncryptedWalletSecrets {
    salt: [u8; SALT_LENGTH],
    nonce: [u8; NONCE_LENGTH],
    ciphertext: Vec<u8>,
    key_derivation_parameters: KeyDerivationParameters,
}

impl EncryptedWalletSecrets {
    pub fn encrypt(plaintext: &[u8], password: &str) -> Result<Self, String> {
        Self::encrypt_with_parameters(plaintext, password, CURRENT_KEY_DERIVATION_PARAMETERS)
    }

    pub(crate) fn encrypt_with_parameters(
        plaintext: &[u8],
        password: &str,
        key_derivation_parameters: KeyDerivationParameters,
    ) -> Result<Self, String> {
        let mut salt = [0_u8; SALT_LENGTH];
        getrandom::fill(&mut salt)
            .map_err(|error| format!("could not generate wallet salt: {error}"))?;

        let encryption_key = derive_encryption_key(password, &salt, key_derivation_parameters)?;

        Self::encrypt_with_key(plaintext, &salt, &encryption_key, key_derivation_parameters)
    }

    pub(crate) fn derive_encryption_key(&self, password: &str) -> Result<EncryptionKey, String> {
        derive_encryption_key(password, &self.salt, self.key_derivation_parameters)
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

    pub(crate) fn decrypt_v1_wallet_with_key(
        &self,
        encryption_key: &EncryptionKey,
    ) -> Result<DecryptedWalletSecrets, String> {
        let mut plaintext =
            decrypt_wallet_secrets_with_key(&self.ciphertext, encryption_key, &self.nonce)?;

        let result = serde_json::from_slice::<LegacyWalletSecretsV1>(&plaintext)
            .map(|legacy| DecryptedWalletSecrets::new(legacy.bip39_entropy))
            .map_err(|error| format!("could not decode version 1 wallet secrets: {error}"));

        plaintext.zeroize();

        result
    }

    fn encrypt_with_key(
        plaintext: &[u8],
        salt: &[u8; SALT_LENGTH],
        encryption_key: &EncryptionKey,
        key_derivation_parameters: KeyDerivationParameters,
    ) -> Result<Self, String> {
        let mut nonce = [0_u8; NONCE_LENGTH];
        getrandom::fill(&mut nonce)
            .map_err(|error| format!("could not generate wallet nonce: {error}"))?;

        let ciphertext = encrypt_wallet_secrets_with_key(plaintext, encryption_key, &nonce)?;

        Ok(Self {
            salt: *salt,
            nonce,
            ciphertext,
            key_derivation_parameters,
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

    pub(crate) fn key_derivation_parameters(&self) -> KeyDerivationParameters {
        self.key_derivation_parameters
    }

    pub(crate) fn from_encrypted_parts(
        salt: [u8; SALT_LENGTH],
        nonce: [u8; NONCE_LENGTH],
        ciphertext: Vec<u8>,
        key_derivation_parameters: KeyDerivationParameters,
    ) -> Result<Self, String> {
        if ciphertext.len() < 16 {
            return Err("wallet ciphertext is too short".to_owned());
        }

        Ok(Self {
            salt,
            nonce,
            ciphertext,
            key_derivation_parameters,
        })
    }
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
    key_derivation_parameters: KeyDerivationParameters,
) -> Result<EncryptionKey, String> {
    let parameters = Params::new(
        key_derivation_parameters.memory_cost_kib(),
        key_derivation_parameters.iterations(),
        key_derivation_parameters.parallelism(),
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
impl DecryptedWalletSecrets {
    pub(crate) fn encrypt_with_parameters(
        &self,
        password: &str,
        key_derivation_parameters: KeyDerivationParameters,
    ) -> Result<EncryptedWalletSecrets, String> {
        let plaintext = self.serialize_for_encryption()?;

        EncryptedWalletSecrets::encrypt_with_parameters(
            &plaintext,
            password,
            key_derivation_parameters,
        )
    }
}

#[cfg(test)]
impl EncryptedWalletSecrets {
    pub fn decrypt(&self, password: &str) -> Result<Vec<u8>, String> {
        let encryption_key = self.derive_encryption_key(password)?;

        decrypt_wallet_secrets_with_key(&self.ciphertext, &encryption_key, &self.nonce)
    }

    pub fn decrypt_wallet(&self, password: &str) -> Result<DecryptedWalletSecrets, String> {
        let encryption_key = self.derive_encryption_key(password)?;

        self.decrypt_wallet_with_key(&encryption_key)
    }
}

#[cfg(test)]
fn encrypt_wallet_secrets(
    plaintext: &[u8],
    password: &str,
    salt: &[u8; SALT_LENGTH],
    nonce: &[u8; NONCE_LENGTH],
) -> Result<Vec<u8>, String> {
    let encryption_key = derive_encryption_key(password, salt, CURRENT_KEY_DERIVATION_PARAMETERS)?;

    encrypt_wallet_secrets_with_key(plaintext, &encryption_key, nonce)
}

#[cfg(test)]
fn decrypt_wallet_secrets(
    ciphertext: &[u8],
    password: &str,
    salt: &[u8; SALT_LENGTH],
    nonce: &[u8; NONCE_LENGTH],
) -> Result<Vec<u8>, String> {
    let encryption_key = derive_encryption_key(password, salt, CURRENT_KEY_DERIVATION_PARAMETERS)?;

    decrypt_wallet_secrets_with_key(ciphertext, &encryption_key, nonce)
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
            .encrypt_with_key(
                encrypted.salt(),
                &encryption_key,
                encrypted.key_derivation_parameters(),
            )
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
        let original = DecryptedWalletSecrets::new([42; 16]);

        let encrypted = original
            .encrypt("correct horse")
            .expect("wallet should encrypt");

        let restored = encrypted
            .decrypt_wallet("correct horse")
            .expect("wallet should decrypt");

        assert_eq!(restored.bip39_entropy, original.bip39_entropy);
    }

    #[test]
    fn version_one_wallet_payload_upgrades_to_entropy_only_secrets() {
        let entropy = [42_u8; 16];
        let legacy_json = serde_json::json!({
            "bip39_entropy": entropy,
            "next_receiving_index": 7,
            "next_change_index": 3,
        });
        let plaintext = serde_json::to_vec(&legacy_json).expect("legacy JSON should serialize");
        let encrypted = EncryptedWalletSecrets::encrypt(&plaintext, "correct horse")
            .expect("legacy wallet should encrypt");
        let encryption_key = encrypted
            .derive_encryption_key("correct horse")
            .expect("key should derive");

        // A version 1 payload must not be accepted as a version 2 payload.
        assert!(encrypted.decrypt_wallet_with_key(&encryption_key).is_err());

        let upgraded = encrypted
            .decrypt_v1_wallet_with_key(&encryption_key)
            .expect("version 1 wallet payload should decrypt");
        assert!(upgraded.matches_entropy(&[42; 16]));
    }
}
