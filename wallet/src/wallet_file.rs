use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

use crate::config::WALLET_NETWORK;
use crate::wallet_secrets::{
    ENCRYPTION_ALGORITHM, EncryptedWalletSecrets, KEY_DERIVATION_ALGORITHM, KeyDerivationParameters,
};

const MAX_WALLET_FILE_BYTES: u64 = 64 * 1024;
const LEGACY_WALLET_FILE_VERSION: u32 = 1;
const WALLET_FILE_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WalletFileVersion {
    V1,
    V2,
}

pub(crate) struct LoadedWalletFile {
    pub(crate) version: WalletFileVersion,
    pub(crate) encrypted_wallet: EncryptedWalletSecrets,
}

pub(crate) fn save_new(
    path: &Path,
    encrypted_wallet: &EncryptedWalletSecrets,
) -> Result<(), String> {
    let json = encode_wallet_file(encrypted_wallet)?;
    if json.len() as u64 > MAX_WALLET_FILE_BYTES {
        return Err("encrypted wallet file is too large".to_owned());
    }
    let temporary_path = temporary_path_for(path)?;
    let result = write_temporary_file(&temporary_path, json.as_bytes()).and_then(|()| {
        // Publishing a new file must fail if it already exists. Unlike rename, hard_link
        // cannot overwrite a wallet created by another app instance during setup.
        fs::hard_link(&temporary_path, path).map_err(|error| {
            format!("Could not create wallet file without overwriting an existing file: {error}")
        })
    });
    let cleanup = fs::remove_file(&temporary_path);
    result?;
    cleanup
        .map_err(|error| format!("Wallet saved, but could not remove its temporary file: {error}"))
}

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

pub(crate) fn load(path: &Path) -> Result<LoadedWalletFile, String> {
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
    let key_derivation_parameters = encrypted_wallet.key_derivation_parameters();
    let wallet_file = SerializedWalletFile {
        version: WALLET_FILE_VERSION,
        network: WALLET_NETWORK.name().to_owned(),
        key_derivation: KeyDerivationSettings {
            algorithm: KEY_DERIVATION_ALGORITHM.to_owned(),
            memory_cost_kib: key_derivation_parameters.memory_cost_kib(),
            iterations: key_derivation_parameters.iterations(),
            parallelism: key_derivation_parameters.parallelism(),
        },
        encryption: ENCRYPTION_ALGORITHM.to_owned(),
        salt: BASE64.encode(encrypted_wallet.salt()),
        nonce: BASE64.encode(encrypted_wallet.nonce()),
        ciphertext: BASE64.encode(encrypted_wallet.ciphertext()),
    };

    serde_json::to_string_pretty(&wallet_file)
        .map_err(|error| format!("could not encode wallet file: {error}"))
}

fn decode_wallet_file(json: &str) -> Result<LoadedWalletFile, String> {
    let wallet_file: SerializedWalletFile = serde_json::from_str(json)
        .map_err(|error| format!("could not decode wallet file: {error}"))?;

    let version = validate_wallet_file(&wallet_file)?;

    let key_derivation_parameters = KeyDerivationParameters::new(
        wallet_file.key_derivation.memory_cost_kib,
        wallet_file.key_derivation.iterations,
        wallet_file.key_derivation.parallelism,
    )?;

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

    let encrypted_wallet = EncryptedWalletSecrets::from_encrypted_parts(
        salt,
        nonce,
        ciphertext,
        key_derivation_parameters,
    )?;

    Ok(LoadedWalletFile {
        version,
        encrypted_wallet,
    })
}

fn validate_wallet_file(wallet_file: &SerializedWalletFile) -> Result<WalletFileVersion, String> {
    let version = match wallet_file.version {
        LEGACY_WALLET_FILE_VERSION => WalletFileVersion::V1,
        WALLET_FILE_VERSION => WalletFileVersion::V2,
        other => return Err(format!("unsupported wallet file version: {other}")),
    };

    if wallet_file.network != WALLET_NETWORK.name() {
        return Err(format!(
            "wallet is for a different network: {}",
            wallet_file.network
        ));
    }

    if wallet_file.key_derivation.algorithm != KEY_DERIVATION_ALGORITHM {
        return Err("unsupported wallet key-derivation algorithm".to_owned());
    }

    if wallet_file.encryption != ENCRYPTION_ALGORITHM {
        return Err("unsupported wallet encryption algorithm".to_owned());
    }

    Ok(version)
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

    use super::{BASE64, WalletFileVersion, decode_wallet_file, encode_wallet_file, load, save};
    use crate::config::WALLET_NETWORK;
    use crate::wallet_secrets::{
        DecryptedWalletSecrets, EncryptedWalletSecrets, KeyDerivationParameters,
    };

    #[test]
    fn encrypted_wallet_has_a_versioned_json_representation() {
        let secrets = DecryptedWalletSecrets::new([42; 16]);
        let encrypted = secrets
            .encrypt("correct horse")
            .expect("wallet should encrypt");

        let json = encode_wallet_file(&encrypted).expect("wallet should encode as JSON");

        assert!(json.contains(r#""version": 2"#));
        assert!(json.contains(&format!(r#""network": "{}""#, WALLET_NETWORK.name())));
        assert!(json.contains(r#""algorithm": "argon2id""#));
        assert!(json.contains(r#""memory_cost_kib": 65536"#));
        assert!(json.contains(r#""iterations": 3"#));
        assert!(json.contains(r#""parallelism": 4"#));
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

        let loaded = decode_wallet_file(&json).expect("wallet JSON should load");
        assert_eq!(loaded.version, WalletFileVersion::V2);
        let restored = loaded
            .encrypted_wallet
            .decrypt_wallet("correct horse")
            .expect("wallet should decrypt");

        assert!(restored.matches_entropy(&[42; 16]));
    }

    #[test]
    fn version_one_wallet_json_decodes_for_upgrade() {
        let entropy = [42_u8; 16];
        let legacy_payload = serde_json::json!({
            "bip39_entropy": entropy,
            "next_receiving_index": 5,
            "next_change_index": 2,
        });
        let encrypted = EncryptedWalletSecrets::encrypt(
            &serde_json::to_vec(&legacy_payload).expect("legacy payload should serialize"),
            "correct horse",
        )
        .expect("legacy wallet should encrypt");
        let json = encode_wallet_file(&encrypted)
            .expect("wallet should encode")
            .replace(r#""version": 2"#, r#""version": 1"#);

        let loaded = decode_wallet_file(&json).expect("version 1 wallet JSON should load");
        assert_eq!(loaded.version, WalletFileVersion::V1);
        let encryption_key = loaded
            .encrypted_wallet
            .derive_encryption_key("correct horse")
            .expect("key should derive");
        let restored = loaded
            .encrypted_wallet
            .decrypt_v1_wallet_with_key(&encryption_key)
            .expect("version 1 secrets should decrypt");
        assert!(restored.matches_entropy(&[42; 16]));
    }

    #[test]
    fn wallet_using_legacy_argon2_parameters_can_be_loaded() {
        let legacy_parameters = KeyDerivationParameters::new(19 * 1024, 2, 1)
            .expect("legacy parameters should remain supported");
        let encrypted = EncryptedWalletSecrets::encrypt_with_parameters(
            b"legacy wallet secrets",
            "correct horse",
            legacy_parameters,
        )
        .expect("legacy wallet should encrypt");
        let json = encode_wallet_file(&encrypted).expect("legacy wallet should encode");

        let loaded = decode_wallet_file(&json).expect("legacy wallet should load");
        let plaintext = loaded
            .encrypted_wallet
            .decrypt("correct horse")
            .expect("legacy wallet should decrypt");

        assert_eq!(plaintext, b"legacy wallet secrets");
        assert_eq!(
            loaded.encrypted_wallet.key_derivation_parameters(),
            legacy_parameters
        );
    }

    #[test]
    fn excessive_argon2_parameters_are_rejected_before_key_derivation() {
        let encrypted = EncryptedWalletSecrets::encrypt(b"wallet secrets", "correct horse")
            .expect("wallet should encrypt");
        let json = encode_wallet_file(&encrypted).expect("wallet should encode");
        let changed_json = json.replace(
            r#""memory_cost_kib": 65536"#,
            r#""memory_cost_kib": 262145"#,
        );

        let error = decode_wallet_file(&changed_json)
            .err()
            .expect("excessive memory setting should be rejected");

        assert_eq!(
            error,
            "wallet Argon2 memory cost is outside the supported range"
        );
    }

    #[test]
    fn unsupported_wallet_version_is_rejected() {
        let secrets = DecryptedWalletSecrets::new([42; 16]);
        let encrypted = secrets
            .encrypt("correct horse")
            .expect("wallet should encrypt");
        let json = encode_wallet_file(&encrypted).expect("wallet should encode");

        for unsupported in [0, 3, 99] {
            let changed_json =
                json.replace(r#""version": 2"#, &format!(r#""version": {unsupported}"#));
            assert!(decode_wallet_file(&changed_json).is_err(), "{unsupported}");
        }
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
        assert!(file_contents.contains(r#""version": 2"#));
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
        assert_eq!(loaded.version, WalletFileVersion::V2);

        let plaintext = loaded
            .encrypted_wallet
            .decrypt("correct horse")
            .expect("loaded wallet should decrypt");

        assert_eq!(plaintext, b"example wallet secrets");
    }

    #[test]
    fn saving_a_new_wallet_never_overwrites_an_existing_file() {
        let directory = tempfile::tempdir().expect("temporary wallet directory");
        let path = directory.path().join("Savings.json");
        let encrypted = EncryptedWalletSecrets::encrypt(b"test secrets", "test wallet password")
            .expect("encrypt test wallet");
        super::save_new(&path, &encrypted).expect("save new wallet");
        let original = std::fs::read(&path).unwrap();
        assert!(super::save_new(&path, &encrypted).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
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
