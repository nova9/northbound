use std::str::FromStr;

use bip39::{Mnemonic, WordCount};
use bitcoin::bip32::{ChildNumber, DerivationPath, Xpriv, Xpub};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use zeroize::Zeroize;

use crate::config::{Network, WALLET_NETWORK};

const P2QRH_PURPOSE: u32 = 88;
const TRUENORTH_MAINNET_COIN_TYPE: u32 = 1867;
const TEST_CHAIN_COIN_TYPE: u32 = 1;
const DEFAULT_ACCOUNT: u32 = 0;
const RECEIVING_BRANCH: u32 = 0;
const CHANGE_BRANCH: u32 = 1;

pub(crate) fn generate_recovery_phrase_and_entropy() -> Result<(String, [u8; 16]), String> {
    let mnemonic = Mnemonic::generate(WordCount::Words12)
        .map_err(|error| format!("could not generate recovery phrase: {error}"))?;

    mnemonic_to_phrase_and_entropy(mnemonic)
}

pub(crate) fn parse_recovery_phrase_and_entropy(
    recovery_phrase: &str,
) -> Result<(String, [u8; 16]), String> {
    let mnemonic = parse_recovery_phrase(recovery_phrase)?;

    mnemonic_to_phrase_and_entropy(mnemonic)
}

pub(crate) fn recovery_phrase_matches_entropy(
    recovery_phrase: &str,
    expected_entropy: &[u8; 16],
) -> Result<bool, String> {
    let mnemonic = parse_recovery_phrase(recovery_phrase)?;

    let mut entered_entropy: [u8; 16] = mnemonic
        .to_entropy()
        .try_into()
        .map_err(|_| "recovery phrase must contain 16 bytes of entropy")?;
    let matches = entered_entropy == *expected_entropy;
    entered_entropy.zeroize();

    Ok(matches)
}

fn parse_recovery_phrase(recovery_phrase: &str) -> Result<Mnemonic, String> {
    let mnemonic = Mnemonic::parse(recovery_phrase)
        .map_err(|error| format!("invalid recovery phrase: {error}"))?;

    if mnemonic.word_count() != 12 {
        return Err("recovery phrase must contain exactly 12 words".to_owned());
    }

    Ok(mnemonic)
}

fn mnemonic_to_phrase_and_entropy(mnemonic: Mnemonic) -> Result<(String, [u8; 16]), String> {
    let entropy: [u8; 16] = mnemonic
        .to_entropy()
        .try_into()
        .map_err(|_| "recovery phrase did not contain 16 bytes of entropy".to_owned())?;

    Ok((mnemonic.to_string(), entropy))
}

#[derive(Clone, Debug)]
pub(crate) struct AccountPublicKeys {
    receiving: Xpub,
    change: Xpub,
}

impl AccountPublicKeys {
    pub(crate) fn from_entropy(entropy: &[u8; 16]) -> Result<Self, String> {
        Self::from_entropy_for_network(entropy, WALLET_NETWORK)
    }

    fn from_entropy_for_network(entropy: &[u8; 16], network: Network) -> Result<Self, String> {
        let mnemonic = Mnemonic::from_entropy(entropy)
            .map_err(|error| format!("invalid wallet entropy: {error}"))?;
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

        let account_path = DerivationPath::from_str(&format!(
            "m/{P2QRH_PURPOSE}'/{coin_type}'/{DEFAULT_ACCOUNT}'"
        ))
        .map_err(|error| format!("invalid derivation path: {error}"))?;

        let secp = Secp256k1::new();
        let account_key = master_key
            .derive_priv(&secp, &account_path)
            .map_err(|error| format!("could not derive wallet account key: {error}"))?;

        let receiving_branch = ChildNumber::from_normal_idx(RECEIVING_BRANCH)
            .map_err(|error| format!("invalid receiving branch index: {error}"))?;
        let change_branch = ChildNumber::from_normal_idx(CHANGE_BRANCH)
            .map_err(|error| format!("invalid change branch index: {error}"))?;

        let receiving_key = account_key
            .derive_priv(&secp, &[receiving_branch])
            .map_err(|error| format!("could not derive receiving branch key: {error}"))?;
        let change_key = account_key
            .derive_priv(&secp, &[change_branch])
            .map_err(|error| format!("could not derive change branch key: {error}"))?;

        Ok(Self {
            receiving: Xpub::from_priv(&secp, &receiving_key),
            change: Xpub::from_priv(&secp, &change_key),
        })
    }

    pub(crate) fn receiving_public_key(&self, address_index: u32) -> Result<[u8; 32], String> {
        derive_branch_child_public_key(&self.receiving, address_index)
    }

    pub(crate) fn change_public_key(&self, address_index: u32) -> Result<[u8; 32], String> {
        derive_branch_child_public_key(&self.change, address_index)
    }
}

fn derive_branch_child_public_key(
    branch_key: &Xpub,
    address_index: u32,
) -> Result<[u8; 32], String> {
    let child_number = ChildNumber::from_normal_idx(address_index)
        .map_err(|_| "wallet address index is exhausted".to_owned())?;
    let secp = Secp256k1::verification_only();
    let child_key = branch_key
        .ckd_pub(&secp, child_number)
        .map_err(|error| format!("could not derive wallet public key: {error}"))?;

    Ok(child_key.to_x_only_pub().serialize())
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
pub(crate) fn derive_regtest_receiving_public_key(
    recovery_phrase: &str,
    address_index: u32,
) -> Result<[u8; 32], String> {
    let mnemonic = Mnemonic::parse(recovery_phrase)
        .map_err(|error| format!("invalid recovery phrase: {error}"))?;

    derive_public_key_from_mnemonic(&mnemonic, RECEIVING_BRANCH, address_index, Network::Regtest)
}

#[cfg(test)]
fn derive_regtest_receiving_public_key_from_entropy(
    entropy: &[u8; 16],
    address_index: u32,
) -> Result<[u8; 32], String> {
    let mnemonic = Mnemonic::from_entropy(entropy)
        .map_err(|error| format!("invalid wallet entropy: {error}"))?;

    derive_public_key_from_mnemonic(&mnemonic, RECEIVING_BRANCH, address_index, Network::Regtest)
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

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
