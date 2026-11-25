use bip39::Mnemonic;

use super::{
    UnlockedWallet, generate_new_wallet, load_and_unlock_wallet, restore_wallet,
    save_new_wallet_and_unlock, save_restored_wallet_and_unlock,
};
use crate::config::{Network, WALLET_NETWORK};
use crate::crypto::{keys, p2qrh};
use crate::server::ReceivingAddress;
use crate::server::discovery::DiscoveredWalletAddresses;
use crate::storage::secrets::{
    CURRENT_KEY_DERIVATION_PARAMETERS, DecryptedWalletSecrets, EncryptedWalletSecrets,
    KeyDerivationParameters,
};
use crate::storage::wallet_file::{self, WalletFileVersion};
use crate::transactions::create_transaction_plan;

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
fn new_wallet_has_a_valid_twelve_word_recovery_phrase() {
    let wallet = generate_new_wallet().expect("new wallet should be generated");

    let mnemonic =
        Mnemonic::parse(wallet.recovery_phrase()).expect("generated phrase should be valid BIP39");

    assert_eq!(mnemonic.word_count(), 12);
}

#[test]
fn generated_recovery_phrase_can_be_confirmed() {
    let wallet = generate_new_wallet().expect("new wallet should be generated");

    let result = wallet
        .confirms_recovery_phrase(wallet.recovery_phrase())
        .expect("generated recovery phrase should be valid");

    assert!(result);
}

#[test]
fn a_different_recovery_phrase_is_not_confirmed() {
    let wallet = generate_new_wallet().expect("new wallet should be generated");
    let other_wallet = generate_new_wallet().expect("second wallet should be generated");

    let result = wallet
        .confirms_recovery_phrase(other_wallet.recovery_phrase())
        .expect("second recovery phrase should be valid");

    assert!(!result);
}

#[test]
fn an_invalid_recovery_phrase_is_rejected() {
    let wallet = generate_new_wallet().expect("new wallet should be generated");

    let result = wallet.confirms_recovery_phrase("this is not a valid recovery phrase");

    assert!(result.is_err());
}

#[test]
fn restored_phrase_produces_the_expected_first_address() {
    let recovery_phrase = "abandon abandon abandon abandon abandon abandon \
     abandon abandon abandon abandon abandon about";

    let restored =
        restore_wallet(recovery_phrase).expect("standard recovery phrase should restore");

    let public_key = keys::derive_regtest_receiving_public_key(restored.recovery_phrase(), 0)
        .expect("restored wallet should derive a key");

    let address = p2qrh::address_from_public_key(&public_key, Network::Regtest)
        .expect("restored key should produce an address");

    assert_eq!(
        address,
        "rnorth1zfgvaxg6parqfv2drvsk9zadefh40zg97ese44hjkjzkjamkftpzsnpx4h6",
    );
}

#[test]
fn unlocking_version_one_wallet_upgrades_file_to_version_two() {
    let directory = tempfile::tempdir().expect("temporary wallet folder");
    let wallet_path = directory.path().join("Legacy.json");
    let pending = generate_new_wallet().expect("wallet should generate");
    let entropy = *pending.secrets().bip39_entropy();

    write_version_one_wallet_file(&wallet_path, entropy, "legacy password", 9, 4);
    assert_eq!(
        wallet_file::load(&wallet_path)
            .expect("v1 file should load")
            .version,
        WalletFileVersion::V1
    );

    let unlocked = load_and_unlock_wallet(&wallet_path, "legacy password")
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

    let reopened = load_and_unlock_wallet(&wallet_path, "legacy password")
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

    assert!(load_and_unlock_wallet(&wallet_path, "wrong password").is_err());
    assert_eq!(
        std::fs::read(&wallet_path).expect("wallet should remain readable"),
        original
    );
}

#[test]
fn wallets_keep_their_passwords_in_separate_files() {
    let directory = tempfile::tempdir().expect("temporary wallet folder");
    let first_path = directory.path().join("Spending.json");
    let second_path = directory.path().join("Savings.json");
    let first_pending = generate_new_wallet().expect("first wallet");
    let second_pending = generate_new_wallet().expect("second wallet");
    let mut first = save_new_wallet_and_unlock(
        &first_path,
        &first_pending,
        first_pending.recovery_phrase(),
        "first wallet password",
        "first wallet password",
    )
    .expect("save first wallet");
    let _second = save_new_wallet_and_unlock(
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

    assert!(load_and_unlock_wallet(&first_path, "first wallet password").is_err());
    assert!(load_and_unlock_wallet(&second_path, "updated first password").is_err());
    let first = load_and_unlock_wallet(&first_path, "updated first password").unwrap();
    let second = load_and_unlock_wallet(&second_path, "second wallet password").unwrap();
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
    let pending = generate_new_wallet().expect("wallet phrase");

    let restored = save_restored_wallet_and_unlock(
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
    let reopened = load_and_unlock_wallet(&restored_path, "restored password").unwrap();
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
    let pending = generate_new_wallet().unwrap();
    std::fs::write(&path, "another wallet was created here").unwrap();

    assert!(
        save_restored_wallet_and_unlock(
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
    let pending_wallet = generate_new_wallet().expect("wallet should be generated");
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
    let pending_wallet = generate_new_wallet().expect("wallet should be generated");
    let other_wallet = generate_new_wallet().expect("second wallet should be generated");
    let other_recovery_phrase = other_wallet.recovery_phrase().to_owned();
    let unlocked_wallet = UnlockedWallet::new(&wallet_path, pending_wallet.secrets().clone())
        .expect("wallet should unlock");

    let phrase_matches = unlocked_wallet
        .confirms_recovery_phrase(&other_recovery_phrase)
        .expect("recovery phrase should be valid");

    assert!(!phrase_matches);
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
    let mut wallet = load_and_unlock_wallet(&wallet_path, "current password")
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
    let mut wallet = load_and_unlock_wallet(&wallet_path, "current password")
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
    let mut wallet = load_and_unlock_wallet(&wallet_path, "current password")
        .expect("test wallet should unlock");

    let result = wallet.change_password_at_path(
        &wallet_path,
        "incorrect password",
        "replacement password",
        "replacement password",
    );

    assert!(result.is_err());
    let saved_wallet = wallet_file::load(&wallet_path).expect("original wallet should still load");
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
fn signing_a_transaction_does_not_modify_the_wallet_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wallet.json");
    let pending = generate_new_wallet().unwrap();
    let wallet = save_new_wallet_and_unlock(
        &path,
        &pending,
        pending.recovery_phrase(),
        "test password",
        "test password",
    )
    .unwrap();
    let scanner = wallet.address_scanner();
    let recipient = scanner.receiving_address_at(0).unwrap();
    let change_address = scanner.change_address_at(0).unwrap();
    let discovered = DiscoveredWalletAddresses {
        receiving_addresses: vec![ReceivingAddress {
            index: 0,
            address: recipient.clone(),
        }],
        change_addresses: vec![change_address.clone()],
    };
    let script = p2qrh::script_pubkey_from_address(&recipient, WALLET_NETWORK).unwrap();
    let utxos = scanner
        .verify_utxos(
            vec![shared::WalletUtxo {
                transaction_id: "ab".repeat(32),
                output_index: 0,
                amount: 100_000,
                script_pubkey: bitcoin::hex::DisplayHex::as_hex(script.as_slice()).to_string(),
                block_height: 1,
                confirmations: 1,
                coinbase: false,
            }],
            &discovered,
        )
        .unwrap();
    let plan = create_transaction_plan(utxos, 50_000, 1_001, change_address).unwrap();
    let file_before = std::fs::read(&path).unwrap();

    let signed = wallet
        .build_and_sign_transaction(&plan, &recipient)
        .unwrap();
    assert!(!signed.transaction_hex.is_empty());
    assert_eq!(signed.transaction_id.len(), 64);
    assert_eq!(std::fs::read(&path).unwrap(), file_before);
}
