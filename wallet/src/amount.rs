use shared::WalletTransaction;

use crate::config::CURRENCY_TICKER;

pub const ATOMIC_UNITS_PER_NORTH: u64 = 100_000_000;

pub fn parse_north(value: &str) -> Result<u64, String> {
    let value = value.trim();
    let (whole_part, fractional_part) = value.split_once('.').unwrap_or((value, ""));

    if whole_part.is_empty() || fractional_part.len() > 8 {
        return Err("Enter an amount with no more than eight decimal places.".to_owned());
    }

    let whole_units = whole_part
        .parse::<u64>()
        .map_err(|_| format!("Enter a valid {CURRENCY_TICKER} amount."))?;
    let fractional_units = if fractional_part.is_empty() {
        0
    } else {
        let fraction = fractional_part
            .parse::<u64>()
            .map_err(|_| format!("Enter a valid {CURRENCY_TICKER} amount."))?;
        fraction * 10_u64.pow((8 - fractional_part.len()) as u32)
    };

    whole_units
        .checked_mul(ATOMIC_UNITS_PER_NORTH)
        .and_then(|units| units.checked_add(fractional_units))
        .ok_or_else(|| "The payment amount is too large.".to_owned())
}

pub fn format_north(amount: u64) -> String {
    let whole_units = amount / ATOMIC_UNITS_PER_NORTH;
    let fractional_units = amount % ATOMIC_UNITS_PER_NORTH;

    format!("{whole_units}.{fractional_units:08}")
}

pub fn format_north_with_ticker(amount: u64) -> String {
    format!("{} {CURRENCY_TICKER}", format_north(amount))
}

pub fn format_optional_north_with_ticker(amount: Option<u64>) -> String {
    match amount {
        Some(amount) => format_north_with_ticker(amount),
        None => "—".to_owned(),
    }
}

pub fn calculate_total_balance(transactions: &[WalletTransaction]) -> Result<u64, String> {
    let mut total = 0_i128;

    for transaction in transactions {
        total = total
            .checked_add(i128::from(transaction.net_amount))
            .ok_or_else(|| "Transaction history balance is too large.".to_owned())?;
    }

    u64::try_from(total).map_err(|_| "Transaction history returned an invalid balance.".to_owned())
}

#[cfg(test)]
mod tests {
    use shared::WalletTransaction;

    use super::{
        CURRENCY_TICKER, calculate_total_balance, format_north, format_north_with_ticker,
        parse_north,
    };

    #[test]
    fn north_amount_is_parsed_without_floating_point() {
        assert_eq!(parse_north("1.25000001"), Ok(125_000_001));
    }

    #[test]
    fn atomic_units_are_formatted_as_north() {
        assert_eq!(format_north(1_025_000_001), "10.25000001");
        assert_eq!(
            format_north_with_ticker(1_025_000_001),
            format!("10.25000001 {CURRENCY_TICKER}")
        );
    }

    #[test]
    fn total_balance_includes_pending_payment_and_its_fee() {
        let pending_send = WalletTransaction {
            transaction_id: "01".repeat(32),
            net_amount: -1_000_000_165,
            network_fee: Some(165),
            coinbase: false,
            block_height: None,
            confirmations: 0,
            timestamp: None,
            received_addresses: Vec::new(),
        };
        let mining_reward = WalletTransaction {
            transaction_id: "02".repeat(32),
            net_amount: 51_200_000_488,
            network_fee: None,
            coinbase: true,
            block_height: Some(26_073),
            confirmations: 802,
            timestamp: None,
            received_addresses: Vec::new(),
        };

        assert_eq!(
            calculate_total_balance(&[pending_send, mining_reward]),
            Ok(50_200_000_323)
        );
    }
}
