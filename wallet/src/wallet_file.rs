use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

use crate::config::WALLET_NETWORK;
use crate::wallet_secrets::{
    ARGON2_ITERATIONS, ARGON2_MEMORY_COST_KIB, ARGON2_PARALLELISM, ENCRYPTION_ALGORITHM,
    EncryptedWalletSecrets, KEY_DERIVATION_ALGORITHM,
};

const MAX_WALLET_FILE_BYTES: u64 = 64 * 1024;
const WALLET_FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SerializedWalletFile {
    version: u32,
    network: String,
    key_derivation: KeyDerivationSettings,
    encryption: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyDerivationSettings {
    algorithm: String,
    memory_cost_kib: u32,
    iterations: u32,
    parallelism: u32,
}

pub fn save(path: &Path, encrypted_wallet: &EncryptedWalletSecrets) -> Result<(), String> {
    let json = encode_wallet_file(encrypted_wallet)?;

    if json.len() as u64 > MAX_WALLET_FILE_BYTES {
        return Err("encrypted wallet file is too large".to_owned());
    }

    let temporary_path = temporary_path_for(path)?;
    let write_result = write_temporary_file(&temporary_path, json.as_bytes());

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("could not replace wallet file: {error}"));
    }

    Ok(())
}

pub fn load(path: &Path) -> Result<EncryptedWalletSecrets, String> {
    let mut file =
        File::open(path).map_err(|error| format!("could not open wallet file: {error}"))?;

    let file_size = file
        .metadata()
        .map_err(|error| format!("could not inspect wallet file: {error}"))?
        .len();

    if file_size > MAX_WALLET_FILE_BYTES {
        return Err("wallet file is too large".to_owned());
    }

    let mut json = String::new();

    file.read_to_string(&mut json)
        .map_err(|error| format!("could not read wallet file: {error}"))?;

    decode_wallet_file(&json)
}

fn encode_wallet_file(encrypted_wallet: &EncryptedWalletSecrets) -> Result<String, String> {
    let wallet_file = SerializedWalletFile {
        version: WALLET_FILE_VERSION,
        network: WALLET_NETWORK.name().to_owned(),
        key_derivation: KeyDerivationSettings {
            algorithm: KEY_DERIVATION_ALGORITHM.to_owned(),
            memory_cost_kib: ARGON2_MEMORY_COST_KIB,
            iterations: ARGON2_ITERATIONS,
            parallelism: ARGON2_PARALLELISM,
        },
        encryption: ENCRYPTION_ALGORITHM.to_owned(),
        salt: BASE64.encode(encrypted_wallet.salt()),
        nonce: BASE64.encode(encrypted_wallet.nonce()),
        ciphertext: BASE64.encode(encrypted_wallet.ciphertext()),
    };

    serde_json::to_string_pretty(&wallet_file)
        .map_err(|error| format!("could not encode wallet file: {error}"))
}

fn decode_wallet_file(json: &str) -> Result<EncryptedWalletSecrets, String> {
    let wallet_file: SerializedWalletFile = serde_json::from_str(json)
        .map_err(|error| format!("could not decode wallet file: {error}"))?;

    validate_wallet_file(&wallet_file)?;

    let salt = BASE64
        .decode(&wallet_file.salt)
        .map_err(|_| "wallet file contains an invalid salt".to_owned())?
        .try_into()
        .map_err(|_| "wallet salt must be exactly 16 bytes".to_owned())?;

    let nonce = BASE64
        .decode(&wallet_file.nonce)
        .map_err(|_| "wallet file contains an invalid nonce".to_owned())?
        .try_into()
        .map_err(|_| "wallet nonce must be exactly 24 bytes".to_owned())?;

    let ciphertext = BASE64
        .decode(&wallet_file.ciphertext)
        .map_err(|_| "wallet file contains invalid ciphertext".to_owned())?;

    EncryptedWalletSecrets::from_encrypted_parts(salt, nonce, ciphertext)
}

fn validate_wallet_file(wallet_file: &SerializedWalletFile) -> Result<(), String> {
    if wallet_file.version != WALLET_FILE_VERSION {
        return Err(format!(
            "unsupported wallet file version: {}",
            wallet_file.version
        ));
    }

    if wallet_file.network != WALLET_NETWORK.name() {
        return Err(format!(
            "wallet is for a different network: {}",
            wallet_file.network
        ));
    }

    if wallet_file.key_derivation.algorithm != KEY_DERIVATION_ALGORITHM {
        return Err("unsupported wallet key-derivation algorithm".to_owned());
    }

    if wallet_file.key_derivation.memory_cost_kib != ARGON2_MEMORY_COST_KIB
        || wallet_file.key_derivation.iterations != ARGON2_ITERATIONS
        || wallet_file.key_derivation.parallelism != ARGON2_PARALLELISM
    {
        return Err("unsupported wallet key-derivation parameters".to_owned());
    }

    if wallet_file.encryption != ENCRYPTION_ALGORITHM {
        return Err("unsupported wallet encryption algorithm".to_owned());
    }

    Ok(())
}

fn temporary_path_for(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| "wallet path must include a file name".to_owned())?;

    let mut random_bytes = [0_u8; 8];
    getrandom::fill(&mut random_bytes)
        .map_err(|error| format!("could not create temporary wallet path: {error}"))?;

    let random_suffix = u64::from_le_bytes(random_bytes);

    let mut temporary_name = OsString::from(file_name);
    temporary_name.push(format!(".tmp-{random_suffix:016x}"));

    Ok(path.with_file_name(temporary_name))
}

fn write_temporary_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(path)
        .map_err(|error| format!("could not create temporary wallet file: {error}"))?;

    file.write_all(contents)
        .map_err(|error| format!("could not write wallet file: {error}"))?;

    file.sync_all()
        .map_err(|error| format!("could not flush wallet file: {error}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::{BASE64, decode_wallet_file, encode_wallet_file, load, save};
    use crate::config::WALLET_NETWORK;
    use crate::wallet_secrets::{DecryptedWalletSecrets, EncryptedWalletSecrets};

    #[test]
    fn encrypted_wallet_has_a_versioned_json_representation() {
        let secrets = DecryptedWalletSecrets::new([42; 16]);
        let encrypted = secrets
            .encrypt("correct horse")
            .expect("wallet should encrypt");

        let json = encode_wallet_file(&encrypted).expect("wallet should encode as JSON");

        assert!(json.contains(r#""version": 1"#));
        assert!(json.contains(&format!(r#""network": "{}""#, WALLET_NETWORK.name())));
        assert!(json.contains(r#""algorithm": "argon2id""#));
        assert!(json.contains(r#""encryption": "xchacha20poly1305""#));
        assert!(!json.contains("bip39_entropy"));
        assert!(!json.contains(&BASE64.encode([42; 16])));
    }

    #[test]
    fn wallet_json_can_be_loaded_and_decrypted() {
        let original = DecryptedWalletSecrets::new([42; 16]);
        let encrypted = original
            .encrypt("correct horse")
            .expect("wallet should encrypt");
        let json = encode_wallet_file(&encrypted).expect("wallet should encode");

        let encrypted = decode_wallet_file(&json).expect("wallet JSON should load");
        let restored = encrypted
            .decrypt_wallet("correct horse")
            .expect("wallet should decrypt");

        assert!(restored.matches_entropy(&[42; 16]));
        assert_eq!(restored.next_receiving_index(), 0);
    }

    #[test]
    fn unsupported_wallet_version_is_rejected() {
        let secrets = DecryptedWalletSecrets::new([42; 16]);
        let encrypted = secrets
            .encrypt("correct horse")
            .expect("wallet should encrypt");
        let json = encode_wallet_file(&encrypted).expect("wallet should encode");
        let changed_json = json.replace(r#""version": 1"#, r#""version": 99"#);

        let result = decode_wallet_file(&changed_json);

        assert!(result.is_err());
    }

    #[test]
    fn encrypted_wallet_can_be_saved_and_loaded() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");

        let wallet_path = directory.path().join("wallet.json");

        let encrypted = EncryptedWalletSecrets::encrypt(b"example wallet secrets", "correct horse")
            .expect("test wallet should encrypt");

        save(&wallet_path, &encrypted).expect("encrypted wallet should save");

        let file_contents =
            std::fs::read_to_string(&wallet_path).expect("saved wallet should be readable");

        assert!(!file_contents.contains("example wallet secrets"));
        assert!(file_contents.contains(r#""version": 1"#));
        assert!(file_contents.contains(r#""ciphertext":"#));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let permissions = std::fs::metadata(&wallet_path)
                .expect("saved wallet should have metadata")
                .permissions()
                .mode()
                & 0o777;

            assert_eq!(permissions, 0o600);
        }

        let loaded = load(&wallet_path).expect("encrypted wallet should load");

        let plaintext = loaded
            .decrypt("correct horse")
            .expect("loaded wallet should decrypt");

        assert_eq!(plaintext, b"example wallet secrets");
    }

    #[test]
    fn oversized_wallet_file_is_rejected() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");

        let wallet_path = directory.path().join("wallet.json");

        std::fs::write(&wallet_path, vec![b'x'; 64 * 1024 + 1])
            .expect("oversized test file should be written");

        let result = load(&wallet_path);

        assert!(result.is_err());
    }
}
