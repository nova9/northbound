use serde_json::Number;

use crate::{Error, Result};

// TrueNorth v30.5.4: src/consensus/amount.h. This is a consensus sanity ceiling.
pub const MAX_MONEY: i64 = 1_000_000_000_000_000_000;

pub fn atomic_units(amount: &Number) -> Result<i64> {
    let text = amount.to_string();
    let invalid = || Error::InvalidData(format!("invalid monetary amount: {text}"));
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => {
            let exponent = exponent.parse::<i32>().map_err(|_| invalid())?;
            (mantissa, exponent)
        }
        None => (text.as_str(), 0),
    };
    if mantissa.starts_with('-') {
        return Err(invalid());
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    let value = digits.parse::<u64>().map_err(|_| invalid())?;
    let fraction_length = i32::try_from(fraction.len()).map_err(|_| invalid())?;
    let scale = exponent
        .checked_sub(fraction_length)
        .and_then(|scale| scale.checked_add(8))
        .ok_or_else(invalid)?;

    let units = if scale >= 0 {
        let multiplier = 10_u64.checked_pow(scale as u32).ok_or_else(invalid)?;
        value.checked_mul(multiplier).ok_or_else(invalid)?
    } else {
        let precision = scale.checked_abs().ok_or_else(invalid)?;
        let divisor = 10_u64.checked_pow(precision as u32).ok_or_else(invalid)?;
        if !value.is_multiple_of(divisor) {
            return Err(invalid());
        }
        value / divisor
    };
    let units = i64::try_from(units).map_err(|_| invalid())?;
    if units > MAX_MONEY {
        return Err(invalid());
    }
    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::atomic_units;

    #[test]
    fn preserves_atomic_precision_and_accepts_consensus_ceiling() {
        for (decimal, expected) in [
            ("512.00000163", 51_200_000_163),
            ("9999999999.00000001", 999_999_999_900_000_001),
            ("10000000000", 1_000_000_000_000_000_000),
            ("1e-8", 1),
            ("0.00000000", 0),
        ] {
            let number = serde_json::from_str(decimal).unwrap();
            assert_eq!(atomic_units(&number).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_negative_fractional_atomic_and_overflowing_amounts() {
        for decimal in ["-1", "0.000000001", "10000000000.00000001", "1e50", "1e-50"] {
            let number = serde_json::from_str(decimal).unwrap();
            assert!(atomic_units(&number).is_err(), "{decimal}");
        }
    }
}
