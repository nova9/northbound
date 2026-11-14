use bech32::{Fe32, Hrp, segwit};

use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct ScriptAddress {
    pub address: String,
    pub script: Vec<u8>,
}

// TrueNorth v30.5.4 src/kernel/chainparams.cpp, including its own regtest
// Base58 prefixes (122/123), which differ from Bitcoin's test prefixes.
fn prefixes(network: &str) -> Result<(&'static str, u8, u8)> {
    match network {
        "main" => Ok(("north", 53, 65)),
        "regtest" => Ok(("rnorth", 122, 123)),
        _ => Err(Error::InvalidRequest(
            "network must be main or regtest".into(),
        )),
    }
}

pub fn decode_addresses(addresses: &[String], network: &str) -> Result<Vec<ScriptAddress>> {
    if addresses.len() > 100 {
        return Err(Error::InvalidRequest(
            "wallet data requests accept at most 100 addresses".into(),
        ));
    }
    let (hrp, pubkey_prefix, script_prefix) = prefixes(network)?;
    let mut scripts = Vec::<ScriptAddress>::new();
    for address in addresses {
        if address.len() > 128 {
            return Err(Error::InvalidRequest("address is too long".into()));
        }
        let script = if let Ok((prefix, version, program)) = segwit::decode(address) {
            if prefix.to_string().to_ascii_lowercase() != hrp {
                return Err(Error::InvalidRequest(
                    "address belongs to a different network".into(),
                ));
            }
            let opcode = if version.to_u8() == 0 {
                0
            } else {
                0x50 + version.to_u8()
            };
            let mut script = vec![opcode, program.len() as u8];
            script.extend_from_slice(&program);
            script
        } else {
            let payload = base58::decode_check(address)
                .map_err(|_| Error::InvalidRequest("invalid address or checksum".into()))?;
            if payload.len() != 21 {
                return Err(Error::InvalidRequest("invalid address payload".into()));
            }
            let mut script = if payload[0] == pubkey_prefix {
                vec![0x76, 0xa9, 0x14]
            } else if payload[0] == script_prefix {
                vec![0xa9, 0x14]
            } else {
                return Err(Error::InvalidRequest(
                    "address belongs to a different network".into(),
                ));
            };
            script.extend_from_slice(&payload[1..]);
            if payload[0] == pubkey_prefix {
                script.extend_from_slice(&[0x88, 0xac]);
            } else {
                script.push(0x87);
            }
            script
        };
        if scripts.iter().any(|entry| entry.script == script) {
            continue;
        }
        let canonical = address_from_script(&script, network)
            .ok_or_else(|| Error::InvalidRequest("could not encode address".into()))?;
        scripts.push(ScriptAddress {
            address: canonical,
            script,
        });
    }
    Ok(scripts)
}

pub fn address_from_script(script: &[u8], network: &str) -> Option<String> {
    let (hrp, pubkey_prefix, script_prefix) = prefixes(network).ok()?;
    if (4..=42).contains(&script.len()) && usize::from(script[1]) == script.len() - 2 {
        let version = match script[0] {
            0 => 0,
            0x51..=0x60 => script[0] - 0x50,
            _ => return None,
        };
        return segwit::encode(
            Hrp::parse(hrp).ok()?,
            Fe32::try_from(version).ok()?,
            &script[2..],
        )
        .ok();
    }
    let (prefix, hash) = if script.len() == 25
        && script[..3] == [0x76, 0xa9, 0x14]
        && script[23..] == [0x88, 0xac]
    {
        (pubkey_prefix, &script[3..23])
    } else if script.len() == 23 && script[..2] == [0xa9, 0x14] && script[22] == 0x87 {
        (script_prefix, &script[2..22])
    } else {
        return None;
    };
    let mut payload = vec![prefix];
    payload.extend_from_slice(hash);
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
