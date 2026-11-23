use std::collections::HashSet;

use shared::WalletUtxo;

use crate::config::WALLET_NETWORK;
use crate::crypto::{keys, p2qrh};
use crate::server::client::ServerClient;
use crate::transactions::{self, VerifiedUtxo, WalletKeychain};

const ADDRESS_DISCOVERY_GAP_LIMIT: u32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivingAddress {
    pub index: u32,
    pub address: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredWalletAddresses {
    pub receiving_addresses: Vec<ReceivingAddress>,
    pub change_addresses: Vec<String>,
}

impl DiscoveredWalletAddresses {
    pub fn active_receiving_address(&self) -> Result<&ReceivingAddress, String> {
        self.receiving_addresses
            .last()
            .ok_or_else(|| "wallet has no receiving address".to_owned())
    }

    pub fn active_change_address(&self) -> Result<&str, String> {
        self.change_addresses
            .last()
            .map(String::as_str)
            .ok_or_else(|| "wallet has no change address".to_owned())
    }

    pub fn tracked_addresses(&self) -> Vec<String> {
        let mut addresses =
            Vec::with_capacity(self.receiving_addresses.len() + self.change_addresses.len());

        for receiving in &self.receiving_addresses {
            addresses.push(receiving.address.clone());
        }

        for change in &self.change_addresses {
            addresses.push(change.clone());
        }

        addresses
    }
}

#[derive(Clone, Debug)]
pub struct WalletAddressScanner {
    public_keys: keys::AccountPublicKeys,
}

impl WalletAddressScanner {
    pub fn from_entropy(entropy: &[u8; 16]) -> Result<Self, String> {
        let public_keys = keys::AccountPublicKeys::from_entropy(entropy)?;
        Ok(Self { public_keys })
    }

    pub(crate) async fn discover_addresses(
        &self,
        server: &ServerClient,
    ) -> Result<DiscoveredWalletAddresses, String> {
        let receiving_addresses = self.discover_receiving_addresses(server).await?;
        let active_change_index = self
            .scan_keychain_for_active_index(server, WalletKeychain::Change)
            .await?;
        let change_addresses = self.derive_change_addresses_through(active_change_index)?;

        Ok(DiscoveredWalletAddresses {
            receiving_addresses,
            change_addresses,
        })
    }

    pub(crate) async fn discover_receiving_addresses(
        &self,
        server: &ServerClient,
    ) -> Result<Vec<ReceivingAddress>, String> {
        let active_receiving_index = self
            .scan_keychain_for_active_index(server, WalletKeychain::Receiving)
            .await?;
        self.derive_receiving_addresses_through(active_receiving_index)
    }

    pub fn receiving_address_at(&self, index: u32) -> Result<String, String> {
        self.address_at(WalletKeychain::Receiving, index)
    }

    pub fn change_address_at(&self, index: u32) -> Result<String, String> {
        self.address_at(WalletKeychain::Change, index)
    }

    pub fn verify_utxos(
        &self,
        utxos: Vec<WalletUtxo>,
        discovered: &DiscoveredWalletAddresses,
    ) -> Result<Vec<VerifiedUtxo>, String> {
        let receiving_indices: Vec<u32> = discovered
            .receiving_addresses
            .iter()
            .map(|receiving| receiving.index)
            .collect();

        transactions::verify_utxos(
            &self.public_keys,
            utxos,
            &receiving_indices,
            discovered.change_addresses.len(),
        )
    }

    async fn scan_keychain_for_active_index(
        &self,
        server: &ServerClient,
        keychain: WalletKeychain,
    ) -> Result<u32, String> {
        let mut batch_start = 0_u32;
        let mut last_used_index = None;

        loop {
            let mut addresses = Vec::with_capacity(ADDRESS_DISCOVERY_GAP_LIMIT as usize);

            for offset in 0..ADDRESS_DISCOVERY_GAP_LIMIT {
                let index = batch_start
                    .checked_add(offset)
                    .ok_or_else(|| "wallet address index is exhausted".to_owned())?;
                addresses.push(self.address_at(keychain, index)?);
            }

            let used_addresses = server.fetch_used_addresses(addresses.clone()).await?;
            let requested_addresses = addresses.iter().cloned().collect::<HashSet<_>>();

            for address in &used_addresses {
                if !requested_addresses.contains(address) {
                    return Err(
                        "wallet server returned activity for an address not requested".to_owned(),
                    );
                }
            }

            for (offset, address) in addresses.iter().enumerate() {
                if used_addresses.contains(address) {
                    let offset = u32::try_from(offset)
                        .map_err(|_| "wallet address scan batch is too large".to_owned())?;
                    last_used_index = Some(
                        batch_start
                            .checked_add(offset)
                            .ok_or_else(|| "wallet address index is exhausted".to_owned())?,
                    );
                }
            }

            let batch_end = batch_start
                .checked_add(ADDRESS_DISCOVERY_GAP_LIMIT - 1)
                .ok_or_else(|| "wallet address index is exhausted".to_owned())?;
            let unused_after_last_used = match last_used_index {
                Some(index) => batch_end - index,
                None => batch_end + 1,
            };

            if unused_after_last_used >= ADDRESS_DISCOVERY_GAP_LIMIT {
                return match last_used_index {
                    Some(index) => index
                        .checked_add(1)
                        .ok_or_else(|| "wallet address index is exhausted".to_owned()),
                    None => Ok(0),
                };
            }

            batch_start = batch_start
                .checked_add(ADDRESS_DISCOVERY_GAP_LIMIT)
                .ok_or_else(|| "wallet address index is exhausted".to_owned())?;
        }
    }

    fn derive_receiving_addresses_through(
        &self,
        last_index: u32,
    ) -> Result<Vec<ReceivingAddress>, String> {
        let mut addresses = Vec::new();

        for index in 0..=last_index {
            let address = self.receiving_address_at(index)?;
            addresses.push(ReceivingAddress { index, address });
        }

        Ok(addresses)
    }

    fn derive_change_addresses_through(&self, last_index: u32) -> Result<Vec<String>, String> {
        let mut addresses = Vec::new();

        for index in 0..=last_index {
            addresses.push(self.change_address_at(index)?);
        }

        Ok(addresses)
    }

    fn address_at(&self, keychain: WalletKeychain, index: u32) -> Result<String, String> {
        let public_key = match keychain {
            WalletKeychain::Receiving => self.public_keys.receiving_public_key(index)?,
            WalletKeychain::Change => self.public_keys.change_public_key(index)?,
        };

        p2qrh::address_from_public_key(&public_key, WALLET_NETWORK)
    }
}

#[cfg(test)]
mod tests {
    use super::WalletAddressScanner;
    use crate::server::client::ServerClient;
    use crate::server::test_support::{index, serve};
    use crate::transactions::WalletKeychain;

    #[test]
    fn address_discovery_finds_first_unused_index_across_gap_batches() {
        let scanner = WalletAddressScanner::from_entropy(&[0_u8; 16]).expect("scanner");
        let addr_0 = scanner.receiving_address_at(0).unwrap();
        let addr_19 = scanner.receiving_address_at(19).unwrap();
        let addr_20 = scanner.receiving_address_at(20).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        // Unused wallet: returns active index 0 in a single batch.
        let (url, server) = serve(1, |_, _| {
            (
                200,
                serde_json::json!({
                    "used_addresses": [],
                    "index": index(),
                }),
            )
        });
        let client = ServerClient::new(&url).unwrap();
        let active_index = runtime
            .block_on(scanner.scan_keychain_for_active_index(&client, WalletKeychain::Receiving))
            .unwrap();
        assert_eq!(active_index, 0);
        server.join().unwrap();

        // Address 0 and 19 used: scans batches 0..20 and 20..40, stops with active index 20.
        let used_set = [addr_0, addr_19.clone()];
        let (url, server) = serve(2, move |_, request| {
            let used = request["addresses"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|value| value.as_str())
                .filter(|address| used_set.iter().any(|used| used == address))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            (
                200,
                serde_json::json!({
                    "used_addresses": used,
                    "index": index(),
                }),
            )
        });
        let client = ServerClient::new(&url).unwrap();
        let active_index = runtime
            .block_on(scanner.scan_keychain_for_active_index(&client, WalletKeychain::Receiving))
            .unwrap();
        assert_eq!(active_index, 20);
        server.join().unwrap();

        // Address 19 and 20 used: scans 0..20, 20..40, and 40..60, stops with active index 21.
        let used_set = [addr_19, addr_20];
        let (url, server) = serve(3, move |_, request| {
            let used = request["addresses"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|value| value.as_str())
                .filter(|address| used_set.iter().any(|used| used == address))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            (
                200,
                serde_json::json!({
                    "used_addresses": used,
                    "index": index(),
                }),
            )
        });
        let client = ServerClient::new(&url).unwrap();
        let active_index = runtime
            .block_on(scanner.scan_keychain_for_active_index(&client, WalletKeychain::Receiving))
            .unwrap();
        assert_eq!(active_index, 21);
        server.join().unwrap();
    }
}
