use bech32::{Fe32, Hrp, segwit};
use sha2::{Digest, Sha256};

const DOMAIN_TAG: &[u8] = b"TrueNorth/QRH/v1";
const SCHNORR_SCHEME_ID: u8 = 0x01;
const EMPTY_SCRIPT_ROOT: [u8; 32] = [0; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    #[cfg_attr(feature = "regtest", allow(dead_code))]
    Mainnet,
    #[cfg_attr(feature = "mainnet", allow(dead_code))]
    Regtest,
}

impl Network {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Regtest => "regtest",
        }
    }

    pub const fn is_test_network(self) -> bool {
        !matches!(self, Self::Mainnet)
    }

    fn address_prefix(self) -> &'static str {
        match self {
            Self::Mainnet => "north",
            Self::Regtest => "bcrt",
        }
    }
}

pub fn address_from_public_key(public_key: &[u8; 32], network: Network) -> Result<String, String> {
    let commitment = commitment_from_public_key(public_key);
    let address_prefix = Hrp::parse(network.address_prefix())
        .map_err(|error| format!("invalid address prefix: {error}"))?;

    segwit::encode(address_prefix, Fe32::Z, &commitment)
        .map_err(|error| format!("could not encode P2QRH address: {error}"))
}

pub fn script_pubkey_from_public_key(public_key: &[u8; 32]) -> [u8; 34] {
    let commitment = commitment_from_public_key(public_key);
    let mut script_pubkey = [0_u8; 34];

    script_pubkey[0] = 0x52;
    script_pubkey[1] = 0x20;
    script_pubkey[2..].copy_from_slice(&commitment);

    script_pubkey
}

pub fn script_pubkey_from_address(address: &str, network: Network) -> Result<[u8; 34], String> {
    let (address_prefix, witness_version, witness_program) =
        segwit::decode(address).map_err(|_| "address is not a valid P2QRH address".to_owned())?;
    let expected_prefix = Hrp::parse(network.address_prefix())
        .map_err(|error| format!("invalid address prefix: {error}"))?;

    if address_prefix != expected_prefix {
        return Err(format!("address is not for {}", network.name()));
    }

    if witness_version != Fe32::Z || witness_program.len() != 32 {
        return Err("address is not a P2QRH address".to_owned());
    }

    let mut script_pubkey = [0_u8; 34];
    script_pubkey[0] = 0x52;
    script_pubkey[1] = 0x20;
    script_pubkey[2..].copy_from_slice(&witness_program);

    Ok(script_pubkey)
}

fn commitment_from_public_key(public_key: &[u8; 32]) -> [u8; 32] {
    calculate_commitment(SCHNORR_SCHEME_ID, public_key, EMPTY_SCRIPT_ROOT)
}

fn calculate_commitment(scheme_id: u8, public_key: &[u8; 32], script_root: [u8; 32]) -> [u8; 32] {
    let mut commitment_hasher = Sha256::new();

    commitment_hasher.update(DOMAIN_TAG);
    commitment_hasher.update([scheme_id]);
    commitment_hasher.update(public_key);
    commitment_hasher.update(script_root);

    let commitment = commitment_hasher.finalize();
    let mut commitment_bytes = [0_u8; 32];
    commitment_bytes.copy_from_slice(&commitment);

    commitment_bytes
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Secp256k1, SecretKey};

    use super::{
        EMPTY_SCRIPT_ROOT, Network, address_from_public_key, calculate_commitment,
        commitment_from_public_key, script_pubkey_from_address, script_pubkey_from_public_key,
    };

    #[test]
    fn commitment_is_deterministic() {
        let public_key = [0xab; 32];

        let first_commitment = commitment_from_public_key(&public_key);
        let second_commitment = commitment_from_public_key(&public_key);

        assert_eq!(first_commitment, second_commitment);
    }

    #[test]
    fn commitment_changes_with_scheme_id() {
        let public_key = [0xab; 32];

        let schnorr_commitment = calculate_commitment(0x01, &public_key, EMPTY_SCRIPT_ROOT);
        let future_scheme_commitment = calculate_commitment(0x02, &public_key, EMPTY_SCRIPT_ROOT);

        assert_ne!(schnorr_commitment, future_scheme_commitment);
    }

    #[test]
    fn commitment_changes_with_public_key() {
        let first_public_key = [0xab; 32];
        let second_public_key = [0xcd; 32];

        let first_commitment = calculate_commitment(0x01, &first_public_key, EMPTY_SCRIPT_ROOT);
        let second_commitment = calculate_commitment(0x01, &second_public_key, EMPTY_SCRIPT_ROOT);

        assert_ne!(first_commitment, second_commitment);
    }

    #[test]
    fn commitment_changes_with_script_root() {
        let public_key = [0xab; 32];
        let mut nonzero_script_root = EMPTY_SCRIPT_ROOT;
        nonzero_script_root[31] = 1;

        let empty_root_commitment = calculate_commitment(0x01, &public_key, EMPTY_SCRIPT_ROOT);
        let nonzero_root_commitment = calculate_commitment(0x01, &public_key, nonzero_script_root);

        assert_ne!(empty_root_commitment, nonzero_root_commitment);
    }

    #[test]
    fn matches_truenorth_core_regtest_address() {
        let public_key_bytes =
            hex::decode("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .expect("reference public key should be valid hex");

        let public_key: [u8; 32] = public_key_bytes
            .try_into()
            .expect("reference public key should contain 32 bytes");

        let address = address_from_public_key(&public_key, Network::Regtest)
            .expect("reference public key should produce an address");

        assert_eq!(
            address,
            "bcrt1zknjh9lq476njy0zngvrvh8w8se0jfc86j7vczt97r8d837kjxelsffa38p",
        );
    }

    #[test]
    fn mainnet_address_uses_the_mainnet_prefix() {
        let public_key = [2_u8; 32];
        let address = address_from_public_key(&public_key, Network::Mainnet)
            .expect("mainnet address should encode");

        assert!(address.starts_with("north1z"));
        assert!(script_pubkey_from_address(&address, Network::Mainnet).is_ok());
        assert!(script_pubkey_from_address(&address, Network::Regtest).is_err());
    }

    #[test]
    fn private_key_one_matches_truenorth_core_regtest_address() {
        let mut private_key_bytes = [0_u8; 32];
        private_key_bytes[31] = 1;

        let secp = Secp256k1::new();

        let private_key =
            SecretKey::from_slice(&private_key_bytes).expect("private key one should be valid");

        let (public_key, _parity) = private_key.x_only_public_key(&secp);
        let public_key_bytes = public_key.serialize();

        assert_eq!(
            hex::encode(public_key_bytes),
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        );

        let address = address_from_public_key(&public_key_bytes, Network::Regtest)
            .expect("reference public key should produce an address");

        assert_eq!(
            address,
            "bcrt1zknjh9lq476njy0zngvrvh8w8se0jfc86j7vczt97r8d837kjxelsffa38p",
        );
    }

    #[test]
    fn p2qrh_script_contains_the_address_commitment() {
        let public_key_bytes =
            hex::decode("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .expect("reference public key should be valid hex");
        let public_key: [u8; 32] = public_key_bytes
            .try_into()
            .expect("reference public key should contain 32 bytes");

        let script_pubkey = script_pubkey_from_public_key(&public_key);

        assert_eq!(script_pubkey[0], 0x52);
        assert_eq!(script_pubkey[1], 0x20);
        assert_eq!(script_pubkey.len(), 34);
    }

    #[test]
    fn p2qrh_address_decodes_to_its_script_pubkey() {
        let address = "bcrt1zknjh9lq476njy0zngvrvh8w8se0jfc86j7vczt97r8d837kjxelsffa38p";
        let script_pubkey = script_pubkey_from_address(address, Network::Regtest)
            .expect("reference address should decode");

        assert_eq!(script_pubkey[0], 0x52);
        assert_eq!(script_pubkey[1], 0x20);
        assert_eq!(script_pubkey.len(), 34);
    }
}
