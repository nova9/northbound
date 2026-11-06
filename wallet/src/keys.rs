use std::str::FromStr;

use bip39::{Mnemonic, WordCount};
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use zeroize::Zeroize;

use crate::config::WALLET_NETWORK;
use crate::p2qrh::Network;
use crate::wallet_secrets::{DecryptedWalletSecrets, EncryptedWalletSecrets};

const P2QRH_PURPOSE: u32 = 88;
const TRUENORTH_MAINNET_COIN_TYPE: u32 = 1867;
const TEST_CHAIN_COIN_TYPE: u32 = 1;
const DEFAULT_ACCOUNT: u32 = 0;
const RECEIVING_BRANCH: u32 = 0;
const CHANGE_BRANCH: u32 = 1;

pub struct PendingWallet {
    recovery_phrase: String,
    secrets: DecryptedWalletSecrets,
}

impl PendingWallet {
    pub fn recovery_phrase(&self) -> &str {
        &self.recovery_phrase
    }

    pub fn confirms_recovery_phrase(&self, entered_phrase: &str) -> Result<bool, String> {
        recovery_phrase_matches_entropy(entered_phrase, self.secrets.bip39_entropy())
    }

    pub(crate) fn encrypt(&self, password: &str) -> Result<EncryptedWalletSecrets, String> {
        self.secrets.encrypt(password)
    }
}

pub(crate) fn recovery_phrase_matches_entropy(
    recovery_phrase: &str,
    expected_entropy: &[u8; 16],
) -> Result<bool, String> {
    let mnemonic = Mnemonic::parse(recovery_phrase)
        .map_err(|error| format!("invalid recovery phrase: {error}"))?;

    if mnemonic.word_count() != 12 {
        return Err("recovery phrase must contain exactly 12 words".to_owned());
    }

    let mut entered_entropy: [u8; 16] = mnemonic
        .to_entropy()
        .try_into()
        .map_err(|_| "recovery phrase must contain 16 bytes of entropy")?;
    let matches = entered_entropy == *expected_entropy;
    entered_entropy.zeroize();

    Ok(matches)
}

impl Drop for PendingWallet {
    fn drop(&mut self) {
        self.recovery_phrase.zeroize();
    }
}

pub fn generate_new_wallet() -> Result<PendingWallet, String> {
    let mnemonic = Mnemonic::generate(WordCount::Words12)
        .map_err(|error| format!("could not generate recovery phrase: {error}"))?;

    wallet_from_mnemonic(mnemonic)
}

pub fn restore_wallet(recovery_phrase: &str) -> Result<PendingWallet, String> {
    let mnemonic = Mnemonic::parse(recovery_phrase)
        .map_err(|error| format!("invalid recovery phrase: {error}"))?;

    if mnemonic.word_count() != 12 {
        return Err("recovery phrase must contain exactly 12 words".to_owned());
    }

    wallet_from_mnemonic(mnemonic)
}

#[cfg(test)]
pub fn derive_regtest_receiving_public_key(
    recovery_phrase: &str,
    address_index: u32,
) -> Result<[u8; 32], String> {
    let mnemonic = Mnemonic::parse(recovery_phrase)
        .map_err(|error| format!("invalid recovery phrase: {error}"))?;

    derive_receiving_public_key_from_mnemonic(&mnemonic, address_index, Network::Regtest)
}

#[cfg(test)]
pub fn derive_regtest_receiving_public_key_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<[u8; 32], String> {
    let mnemonic = Mnemonic::from_entropy(entropy)
        .map_err(|error| format!("invalid wallet entropy: {error}"))?;

    derive_receiving_public_key_from_mnemonic(&mnemonic, address_index, Network::Regtest)
}

pub fn derive_receiving_public_key_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<[u8; 32], String> {
    let mnemonic = Mnemonic::from_entropy(entropy)
        .map_err(|error| format!("invalid wallet entropy: {error}"))?;

    derive_public_key_from_mnemonic(&mnemonic, RECEIVING_BRANCH, address_index, WALLET_NETWORK)
}

pub fn derive_change_public_key_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<[u8; 32], String> {
    let mnemonic = Mnemonic::from_entropy(entropy)
        .map_err(|error| format!("invalid wallet entropy: {error}"))?;

    derive_public_key_from_mnemonic(&mnemonic, CHANGE_BRANCH, address_index, WALLET_NETWORK)
}

pub(crate) fn derive_receiving_private_key_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<SecretKey, String> {
    let mnemonic = Mnemonic::from_entropy(entropy)
        .map_err(|error| format!("invalid wallet entropy: {error}"))?;

    derive_private_key_from_mnemonic(&mnemonic, RECEIVING_BRANCH, address_index, WALLET_NETWORK)
}

pub(crate) fn derive_change_private_key_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<SecretKey, String> {
    let mnemonic = Mnemonic::from_entropy(entropy)
        .map_err(|error| format!("invalid wallet entropy: {error}"))?;

    derive_private_key_from_mnemonic(&mnemonic, CHANGE_BRANCH, address_index, WALLET_NETWORK)
}

#[cfg(test)]
fn derive_receiving_public_key_from_mnemonic(
    mnemonic: &Mnemonic,
    address_index: u32,
    network: Network,
) -> Result<[u8; 32], String> {
    derive_public_key_from_mnemonic(mnemonic, RECEIVING_BRANCH, address_index, network)
}

fn derive_public_key_from_mnemonic(
    mnemonic: &Mnemonic,
    branch: u32,
    address_index: u32,
    network: Network,
) -> Result<[u8; 32], String> {
    let private_key = derive_private_key_from_mnemonic(mnemonic, branch, address_index, network)?;
    let secp = Secp256k1::new();
    let (public_key, _parity) = private_key.x_only_public_key(&secp);

    Ok(public_key.serialize())
}

fn derive_private_key_from_mnemonic(
    mnemonic: &Mnemonic,
    branch: u32,
    address_index: u32,
    network: Network,
) -> Result<SecretKey, String> {
    let mut seed = mnemonic.to_seed("");

    let bip32_network = match network {
        Network::Mainnet => bitcoin::Network::Bitcoin,
        Network::Regtest => bitcoin::Network::Regtest,
    };

    let master_key_result = Xpriv::new_master(bip32_network, &seed);
    seed.zeroize();

    let master_key =
        master_key_result.map_err(|error| format!("could not create master key: {error}"))?;

    let coin_type = match network {
        Network::Mainnet => TRUENORTH_MAINNET_COIN_TYPE,
        Network::Regtest => TEST_CHAIN_COIN_TYPE,
    };

    let path_text =
        format!("m/{P2QRH_PURPOSE}'/{coin_type}'/{DEFAULT_ACCOUNT}'/{branch}/{address_index}");
    let path = DerivationPath::from_str(&path_text)
        .map_err(|error| format!("invalid derivation path: {error}"))?;

    let secp = Secp256k1::new();

    let derived_key = master_key
        .derive_priv(&secp, &path)
        .map_err(|error| format!("could not derive wallet key: {error}"))?;

    Ok(derived_key.private_key)
}

fn wallet_from_mnemonic(mnemonic: Mnemonic) -> Result<PendingWallet, String> {
    let entropy: [u8; 16] = mnemonic
        .to_entropy()
        .try_into()
        .map_err(|_| "recovery phrase did not contain 16 bytes of entropy".to_owned())?;

    let recovery_phrase = mnemonic.to_string();
    let secrets = DecryptedWalletSecrets::new(entropy);

    Ok(PendingWallet {
        recovery_phrase,
        secrets,
    })
}

#[cfg(test)]
mod tests {
    use crate::p2qrh::{Network, address_from_public_key};
    use bip39::Mnemonic;

    use super::{
        derive_change_public_key_from_entropy, derive_receiving_public_key_from_entropy,
        derive_regtest_receiving_public_key, derive_regtest_receiving_public_key_from_entropy,
        generate_new_wallet, restore_wallet,
    };

    #[test]
    fn standard_mnemonic_produces_expected_seed() {
        let recovery_phrase = "abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon about";

        let mnemonic = Mnemonic::parse(recovery_phrase).expect("BIP39 test phrase should be valid");

        let seed = mnemonic.to_seed("");

        assert_eq!(
            hex::encode(seed),
            concat!(
                "5eb00bbddcf069084889a8ab91555681",
                "65f5c453ccb85e70811aaed6f6da5fc19",
                "a5ac40b389cd370d086206dec8aa6c43d",
                "aea6690f20ad3d8d48b2d2ce9e38e4",
            ),
        );
    }

    #[test]
    fn first_regtest_receiving_address_matches_truenorth_core() {
        let recovery_phrase = "abandon abandon abandon abandon abandon abandon \
         abandon abandon abandon abandon abandon about";

        let public_key = derive_regtest_receiving_public_key(recovery_phrase, 0)
            .expect("test phrase should derive a receiving key");

        assert_eq!(
            hex::encode(public_key),
            "aa8b07706ab9645159afff8ea478f616c1925a363de097fc73718764f4117221",
        );

        let address = address_from_public_key(&public_key, Network::Regtest)
            .expect("derived public key should produce an address");

        assert_eq!(
            address,
            "bcrt1zfgvaxg6parqfv2drvsk9zadefh40zg97ese44hjkjzkjamkftpzs5zjqww",
        );
    }

    #[test]
    fn new_wallet_has_a_valid_twelve_word_recovery_phrase() {
        let wallet = generate_new_wallet().expect("new wallet should be generated");

        let mnemonic = Mnemonic::parse(wallet.recovery_phrase())
            .expect("generated phrase should be valid BIP39");

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
    fn entropy_derives_the_same_first_receiving_key() {
        let public_key = derive_regtest_receiving_public_key_from_entropy(&[0; 16], 0)
            .expect("standard test entropy should derive a key");

        assert_eq!(
            hex::encode(public_key),
            "aa8b07706ab9645159afff8ea478f616c1925a363de097fc73718764f4117221",
        );
    }

    #[test]
    fn change_branch_derives_a_different_key() {
        let receiving_key = derive_receiving_public_key_from_entropy(&[0; 16], 0)
            .expect("receiving key should derive");
        let change_key =
            derive_change_public_key_from_entropy(&[0; 16], 0).expect("change key should derive");

        assert_ne!(change_key, receiving_key);
    }

    #[test]
    fn restored_phrase_produces_the_expected_first_address() {
        let recovery_phrase = "abandon abandon abandon abandon abandon abandon \
         abandon abandon abandon abandon abandon about";

        let restored =
            restore_wallet(recovery_phrase).expect("standard recovery phrase should restore");

        let public_key = derive_regtest_receiving_public_key(restored.recovery_phrase(), 0)
            .expect("restored wallet should derive a key");

        let address = address_from_public_key(&public_key, Network::Regtest)
            .expect("restored key should produce an address");

        assert_eq!(
            address,
            "bcrt1zfgvaxg6parqfv2drvsk9zadefh40zg97ese44hjkjzkjamkftpzs5zjqww",
        );
    }
}
