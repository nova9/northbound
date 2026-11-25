use std::collections::HashSet;

use shared::{
    AddressActivityRequest, AddressActivityResponse, DEFAULT_UTXO_PAGE_SIZE, IndexStatus,
    MAX_UTXO_CURSOR_LENGTH, UtxoRequest, UtxoResponse, WalletUtxo,
};

use crate::config::MAX_ADDRESSES_PER_REQUEST;
use crate::server::client::{ServerClient, require_current_index, require_success};

impl ServerClient {
    pub(crate) async fn fetch_used_addresses(
        &self,
        addresses: Vec<String>,
    ) -> Result<Vec<String>, String> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let addresses = unique_addresses(addresses);
        let url = self.endpoint("/v1/addresses/activity");
        let mut snapshot = None;
        let mut used_addresses = Vec::new();
        let mut seen_addresses = HashSet::new();

        for batch in addresses.chunks(MAX_ADDRESSES_PER_REQUEST) {
            let request = AddressActivityRequest {
                addresses: batch.to_vec(),
            };
            let response = self
                .http()
                .post(&url)
                .json(&request)
                .send()
                .await
                .map_err(|error| format!("could not contact wallet server: {error}"))?;
            let response = require_success(response, "Could not load address activity").await?;
            let response = response
                .json::<AddressActivityResponse>()
                .await
                .map_err(|error| {
                    format!("wallet server returned invalid address activity: {error}")
                })?;
            require_matching_snapshot(&mut snapshot, &response.index, addresses.len())?;
            for address in response.used_addresses {
                if !batch.contains(&address) {
                    return Err(
                        "The wallet server returned activity for an unrequested address.".into(),
                    );
                }
                if seen_addresses.insert(address.clone()) {
                    used_addresses.push(address);
                }
            }
        }
        Ok(used_addresses)
    }

    pub(crate) async fn fetch_utxos(
        &self,
        addresses: Vec<String>,
    ) -> Result<Vec<WalletUtxo>, String> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let addresses = unique_addresses(addresses);
        let url = self.endpoint("/v1/utxos");
        let mut snapshot = None;
        let mut utxos = Vec::new();
        let mut seen_outpoints = HashSet::new();

        for batch in addresses.chunks(MAX_ADDRESSES_PER_REQUEST) {
            let mut cursor = None;
            let mut seen_cursors = HashSet::new();
            let mut previous_output = None;
            loop {
                let request = UtxoRequest {
                    addresses: batch.to_vec(),
                    cursor,
                    limit: Some(DEFAULT_UTXO_PAGE_SIZE),
                };
                let response = self
                    .http()
                    .post(&url)
                    .json(&request)
                    .send()
                    .await
                    .map_err(|error| format!("could not contact wallet server: {error}"))?;
                if response.status() == reqwest::StatusCode::CONFLICT {
                    return Err("Spendable outputs changed while loading. Please refresh.".into());
                }
                let response =
                    require_success(response, "Could not load spendable outputs").await?;
                let response = response.json::<UtxoResponse>().await.map_err(|error| {
                    format!("wallet server returned invalid spendable outputs: {error}")
                })?;
                require_matching_snapshot(&mut snapshot, &response.index, addresses.len())?;
                if (request.cursor.is_some() || response.next_cursor.is_some())
                    && response.index.pending_revision.is_none()
                {
                    return Err(
                        "The wallet server must be updated to support paginated outputs.".into(),
                    );
                }
                if response.utxos.len() > usize::from(DEFAULT_UTXO_PAGE_SIZE)
                    || response
                        .next_cursor
                        .as_ref()
                        .is_some_and(|cursor| cursor.len() > MAX_UTXO_CURSOR_LENGTH)
                    || (response.utxos.is_empty() && response.next_cursor.is_some())
                {
                    return Err(
                        "The wallet server returned an invalid spendable output page.".into(),
                    );
                }
                for utxo in response.utxos {
                    let outpoint = (utxo.transaction_id.clone(), utxo.output_index);
                    if !seen_outpoints.insert(outpoint) {
                        return Err(
                            "The wallet server returned a duplicate spendable output.".into()
                        );
                    }
                    let position = (
                        utxo.block_height,
                        utxo.transaction_id.clone(),
                        utxo.output_index,
                    );
                    if previous_output
                        .as_ref()
                        .is_some_and(|previous| previous >= &position)
                    {
                        return Err(
                            "The wallet server returned spendable outputs out of order.".into()
                        );
                    }
                    previous_output = Some(position);
                    utxos.push(utxo);
                }
                cursor = response.next_cursor;
                let Some(next_cursor) = &cursor else {
                    break;
                };
                if !seen_cursors.insert(next_cursor.clone()) {
                    return Err("The wallet server repeated a spendable output cursor.".into());
                }
            }
        }
        Ok(utxos)
    }
}

pub(super) fn unique_addresses(addresses: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for address in addresses {
        if seen.insert(address.clone()) {
            unique.push(address);
        }
    }
    unique
}

fn require_matching_snapshot(
    first: &mut Option<IndexStatus>,
    current: &IndexStatus,
    address_count: usize,
) -> Result<(), String> {
    require_current_index(current)?;
    if address_count > MAX_ADDRESSES_PER_REQUEST && current.pending_revision.is_none() {
        return Err(
            "The wallet server must be updated to support batched address requests.".into(),
        );
    }
    if let Some(first) = first {
        // Timestamps can repeat within one second. The revision identifies the
        // pending snapshot, which determines which confirmed outputs are spendable.
        if first.indexed_height != current.indexed_height
            || first.indexed_hash != current.indexed_hash
            || first.pending_revision != current.pending_revision
        {
            return Err("Wallet data changed while loading. Please refresh.".into());
        }
    } else {
        *first = Some(current.clone());
    }
    Ok(())
}

#[cfg(test)]
#[path = "addresses_tests.rs"]
mod tests;
