use bech32::{Fe32, Hrp, segwit};

use crate::{Error, Result};

const MAX_ADDRESSES_PER_REQUEST: usize = 100;
const MAX_ADDRESS_LENGTH: usize = 128;

// Standard script opcodes used to build and recognize locking scripts.
const OP_0: u8 = 0x00;
const OP_1: u8 = 0x51;
const OP_16: u8 = 0x60;
const OP_DUP: u8 = 0x76;
const OP_EQUAL: u8 = 0x87;
const OP_EQUALVERIFY: u8 = 0x88;
const OP_HASH160: u8 = 0xa9;
const OP_CHECKSIG: u8 = 0xac;
const HASH160_LENGTH: u8 = 20;

#[derive(Clone, Debug)]
pub struct ScriptAddress {
    pub address: String,
    pub script: Vec<u8>,
}

struct NetworkPrefixes {
    bech32_hrp: &'static str,
    p2pkh_prefix: u8,
    p2sh_prefix: u8,
}

// Prefixes from TrueNorth v30.5.4 `src/kernel/chainparams.cpp`.
// Note that regtest Base58 prefixes (122/123) differ from Bitcoin's test prefixes.
fn network_prefixes(network: &str) -> Result<NetworkPrefixes> {
    match network {
        "main" => Ok(NetworkPrefixes {
            bech32_hrp: "north",
            p2pkh_prefix: 53,
            p2sh_prefix: 65,
        }),
        "regtest" => Ok(NetworkPrefixes {
            bech32_hrp: "rnorth",
            p2pkh_prefix: 122,
            p2sh_prefix: 123,
        }),
        _ => Err(Error::InvalidRequest(
            "network must be main or regtest".into(),
        )),
    }
}

/// Decodes and deduplicates wallet addresses into their locking scripts.
pub fn decode_addresses(addresses: &[String], network: &str) -> Result<Vec<ScriptAddress>> {
    if addresses.len() > MAX_ADDRESSES_PER_REQUEST {
        return Err(Error::InvalidRequest(
            "wallet data requests accept at most 100 addresses".into(),
        ));
    }

    let prefixes = network_prefixes(network)?;
    let mut results = Vec::new();

    for address in addresses {
        if address.len() > MAX_ADDRESS_LENGTH {
            return Err(Error::InvalidRequest("address is too long".into()));
        }

        let script = decode_single_address(address, &prefixes)?;
        if results
            .iter()
            .any(|entry: &ScriptAddress| entry.script == script)
        {
            continue;
        }

        let canonical = address_from_script(&script, network)
            .ok_or_else(|| Error::InvalidRequest("could not encode address".into()))?;
        results.push(ScriptAddress {
            address: canonical,
            script,
        });
    }

    Ok(results)
}

/// Encodes a standard witness (including P2QRH), P2PKH, or P2SH script into an address.
pub fn address_from_script(script: &[u8], network: &str) -> Option<String> {
    let prefixes = network_prefixes(network).ok()?;

    if let Some(address) = encode_witness_script(script, prefixes.bech32_hrp) {
        return Some(address);
    }

    encode_legacy_script(script, &prefixes)
}

fn decode_single_address(address: &str, prefixes: &NetworkPrefixes) -> Result<Vec<u8>> {
    if let Ok((hrp, version, program)) = segwit::decode(address) {
        return witness_program_to_script(&hrp, version, &program, prefixes.bech32_hrp);
    }

    decode_base58_address(address, prefixes)
}

fn witness_program_to_script(
    hrp: &Hrp,
    version: Fe32,
    program: &[u8],
    expected_hrp: &str,
) -> Result<Vec<u8>> {
    if hrp.to_string().to_ascii_lowercase() != expected_hrp {
        return Err(Error::InvalidRequest(
            "address belongs to a different network".into(),
        ));
    }

    let version_byte = version.to_u8();
    let version_opcode = if version_byte == 0 {
        OP_0
    } else {
        (OP_1 - 1) + version_byte
    };

    let mut script = Vec::with_capacity(2 + program.len());
    script.push(version_opcode);
    script.push(program.len() as u8);
    script.extend_from_slice(program);
    Ok(script)
}

fn decode_base58_address(address: &str, prefixes: &NetworkPrefixes) -> Result<Vec<u8>> {
    let payload = base58::decode_check(address)
        .map_err(|_| Error::InvalidRequest("invalid address or checksum".into()))?;

    // Base58Check payload is 1-byte version prefix + 20-byte hash160.
    if payload.len() != 21 {
        return Err(Error::InvalidRequest("invalid address payload".into()));
    }

    let prefix = payload[0];
    let hash160 = &payload[1..];

    if prefix == prefixes.p2pkh_prefix {
        // P2PKH: OP_DUP OP_HASH160 <20-byte hash> OP_EQUALVERIFY OP_CHECKSIG
        let mut script = vec![OP_DUP, OP_HASH160, HASH160_LENGTH];
        script.extend_from_slice(hash160);
        script.extend_from_slice(&[OP_EQUALVERIFY, OP_CHECKSIG]);
        Ok(script)
    } else if prefix == prefixes.p2sh_prefix {
        // P2SH: OP_HASH160 <20-byte hash> OP_EQUAL
        let mut script = vec![OP_HASH160, HASH160_LENGTH];
        script.extend_from_slice(hash160);
        script.push(OP_EQUAL);
        Ok(script)
    } else {
        Err(Error::InvalidRequest(
            "address belongs to a different network".into(),
        ))
    }
}

fn encode_witness_script(script: &[u8], expected_hrp: &str) -> Option<String> {
    // Witness scripts are: <version opcode> <program length (2..=40)> <program bytes>.
    if !(4..=42).contains(&script.len()) {
        return None;
    }

    let program_length = usize::from(script[1]);
    if program_length != script.len() - 2 {
        return None;
    }

    let version = match script[0] {
        OP_0 => 0,
        OP_1..=OP_16 => script[0] - (OP_1 - 1),
        _ => return None,
    };

    let hrp = Hrp::parse(expected_hrp).ok()?;
    let version = Fe32::try_from(version).ok()?;
    segwit::encode(hrp, version, &script[2..]).ok()
}

fn encode_legacy_script(script: &[u8], prefixes: &NetworkPrefixes) -> Option<String> {
    let is_p2pkh = script.len() == 25
        && script[..3] == [OP_DUP, OP_HASH160, HASH160_LENGTH]
        && script[23..] == [OP_EQUALVERIFY, OP_CHECKSIG];

    let is_p2sh =
        script.len() == 23 && script[..2] == [OP_HASH160, HASH160_LENGTH] && script[22] == OP_EQUAL;

    let (prefix, hash160) = if is_p2pkh {
        (prefixes.p2pkh_prefix, &script[3..23])
    } else if is_p2sh {
        (prefixes.p2sh_prefix, &script[2..22])
    } else {
        return None;
    };

    let mut payload = Vec::with_capacity(21);
    payload.push(prefix);
    payload.extend_from_slice(hash160);
    Some(base58::encode_check(&payload))
}

#[cfg(test)]
mod tests {
    use super::{address_from_script, decode_addresses};

    #[test]
    fn address_decoding_checks_network_checksum_and_legacy_scripts() {
        let script =
            hex::decode("5220d57d900d942200b8431ba4480f50edc29ca45e1ab476c5be60e628fe7b2ec551")
                .unwrap();
        let address = "rnorth1z647eqrv5ygqtsscm53yq758dc2w2ghs6k3mvt0nquc50u7ewc4gs0engmj";
        assert_eq!(
            decode_addresses(&[address.into()], "regtest").unwrap()[0].script,
            script
        );
        assert!(decode_addresses(&[address.into()], "main").is_err());
        assert!(
            decode_addresses(&[format!("{}q", &address[..address.len() - 1])], "regtest").is_err()
        );
        for network in ["main", "regtest"] {
            for script in [
                format!("76a914{}88ac", "ab".repeat(20)),
                format!("a914{}87", "cd".repeat(20)),
            ] {
                let script = hex::decode(script).unwrap();
                let address = address_from_script(&script, network).unwrap();
                assert_eq!(
                    decode_addresses(&[address], network).unwrap()[0].script,
                    script
                );
            }
        }
    }
}
