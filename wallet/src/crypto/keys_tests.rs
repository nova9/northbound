use bip39::Mnemonic;

use super::{
    AccountPublicKeys, CHANGE_BRANCH, RECEIVING_BRANCH, derive_change_public_key_from_entropy,
    derive_public_key_from_mnemonic, derive_receiving_public_key_from_entropy,
    derive_regtest_receiving_public_key, derive_regtest_receiving_public_key_from_entropy,
};
use crate::config::Network;
use crate::crypto::p2qrh::{address_from_public_key, script_pubkey_from_public_key};

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
        "rnorth1zfgvaxg6parqfv2drvsk9zadefh40zg97ese44hjkjzkjamkftpzsnpx4h6",
    );
}

// Fixed examples for the public BIP39 test phrase and an empty passphrase.
// Source: https://github.com/severka-monitor/truenorth-cold-address/blob/497f0c81fdc4e71d0c17143e79ad56dd0b6c243d/vectors/p2qrh-test-vectors.json
#[test]
fn mainnet_receiving_index_two_has_expected_key_script_and_address() {
    let mnemonic =
        Mnemonic::from_entropy(&[0; 16]).expect("standard test entropy should produce a mnemonic");
    let public_key =
        derive_public_key_from_mnemonic(&mnemonic, RECEIVING_BRANCH, 2, Network::Mainnet)
            .expect("test mnemonic should derive a public key");

    assert_eq!(
        hex::encode(public_key),
        "5a0b786673fa1d37eb3027d960a352f6638ac07f6a408edc2eba330169b13862",
    );
    assert_eq!(
        hex::encode(script_pubkey_from_public_key(&public_key)),
        "5220b464ee5141cc6f48666dbb83bf6757fa605d53c2fad2361920ce6a837d125539",
    );

    let address = address_from_public_key(&public_key, Network::Mainnet)
        .expect("derived public key should produce an address");
    assert_eq!(
        address,
        "north1zk3jwu52pe3h5sendhwpm7e6hlfs9657zltfrvxfqee4gxlgj25usu60dh3",
    );
}

#[test]
fn mainnet_change_index_one_has_expected_key_script_and_address() {
    let mnemonic =
        Mnemonic::from_entropy(&[0; 16]).expect("standard test entropy should produce a mnemonic");
    let public_key = derive_public_key_from_mnemonic(&mnemonic, CHANGE_BRANCH, 1, Network::Mainnet)
        .expect("test mnemonic should derive a public key");

    assert_eq!(
        hex::encode(public_key),
        "290e5f75006abfb9d99d470b1ab1a5d3a90dc9a5d2bd0eecfba12e39f6a36f70",
    );
    assert_eq!(
        hex::encode(script_pubkey_from_public_key(&public_key)),
        "52201a81e2faed49470e9bb2d0e5810c11b13026bbbdaa45d2259a7d740f2800e0f7",
    );

    let address = address_from_public_key(&public_key, Network::Mainnet)
        .expect("derived public key should produce an address");
    assert_eq!(
        address,
        "north1zr2q797hdf9rsaxaj6rjczrq3kyczdwaa4fzayfv6046q72qqurmsp6788r",
    );
}

#[test]
fn regtest_receiving_index_two_has_expected_key_script_and_address() {
    let mnemonic =
        Mnemonic::from_entropy(&[0; 16]).expect("standard test entropy should produce a mnemonic");
    let public_key =
        derive_public_key_from_mnemonic(&mnemonic, RECEIVING_BRANCH, 2, Network::Regtest)
            .expect("test mnemonic should derive a public key");

    assert_eq!(
        hex::encode(public_key),
        "ede473a80677e7b93bf1d1c3a2b775fefa4e1e80d47d95d1c6793916249587c5",
    );
    assert_eq!(
        hex::encode(script_pubkey_from_public_key(&public_key)),
        "5220977b680678dadc6c2f6b5f831af475900cd9e607d0b549e66f870bc89cf9d110",
    );

    let address = address_from_public_key(&public_key, Network::Regtest)
        .expect("derived public key should produce an address");
    assert_eq!(
        address,
        "rnorth1zjaakspncmtwxctmtt7p34ar4jqxdnes86z65nen0su9u388e6ygqwdwxjz",
    );
}

#[test]
fn regtest_change_index_one_has_expected_key_script_and_address() {
    let mnemonic =
        Mnemonic::from_entropy(&[0; 16]).expect("standard test entropy should produce a mnemonic");
    let public_key = derive_public_key_from_mnemonic(&mnemonic, CHANGE_BRANCH, 1, Network::Regtest)
        .expect("test mnemonic should derive a public key");

    assert_eq!(
        hex::encode(public_key),
        "6f9522fc04b2e0650af589e3b5f4c8eb6e6e75849151444ec82d722decfee9be",
    );
    assert_eq!(
        hex::encode(script_pubkey_from_public_key(&public_key)),
        "5220c6105b470a192ee99beffc2ad3259f1bcc3c1b3cdf9ea604e343a494e3445dfd",
    );

    let address = address_from_public_key(&public_key, Network::Regtest)
        .expect("derived public key should produce an address");
    assert_eq!(
        address,
        "rnorth1zccg9k3c2ryhwnxl0ls4dxfvlr0xrcxeum702vp8rgwjffc6yth7sw8xh5z",
    );
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
    let receiving_key =
        derive_receiving_public_key_from_entropy(&[0; 16], 0).expect("receiving key should derive");
    let change_key =
        derive_change_public_key_from_entropy(&[0; 16], 0).expect("change key should derive");

    assert_ne!(change_key, receiving_key);
}

#[test]
fn account_public_keys_match_mnemonic_derivation_across_branches_and_networks() {
    let entropy = [0_u8; 16];
    let mnemonic =
        Mnemonic::from_entropy(&entropy).expect("standard test entropy should produce a mnemonic");

    for network in [Network::Mainnet, Network::Regtest] {
        let public_keys = AccountPublicKeys::from_entropy_for_network(&entropy, network)
            .expect("account public keys should derive");

        for index in [0, 1, 2, 19, 25] {
            let expected_receiving =
                derive_public_key_from_mnemonic(&mnemonic, RECEIVING_BRANCH, index, network)
                    .expect("receiving key should derive");
            let expected_change =
                derive_public_key_from_mnemonic(&mnemonic, CHANGE_BRANCH, index, network)
                    .expect("change key should derive");

            assert_eq!(
                public_keys
                    .receiving_public_key(index)
                    .expect("xpub receiving key should derive"),
                expected_receiving
            );
            assert_eq!(
                public_keys
                    .change_public_key(index)
                    .expect("xpub change key should derive"),
                expected_change
            );
        }
    }
}
