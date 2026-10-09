use thiserror::Error;

#[derive(Error, Debug)]
pub enum GasError {
    #[error("Calculated fee {calculated_gwei:.2} Gwei exceeds configured ceiling of {ceiling_gwei:.2} Gwei")]
    ExceedsCeiling {
        calculated_gwei: f64,
        ceiling_gwei: f64,
    },
    #[error("Arithmetic overflow in gas fee calculation")]
    Overflow,
}

pub struct GasEngine {
    max_fee_ceiling_wei: u128,
    default_priority_fee_wei: u128,
    bump_percent: u32,
}

impl GasEngine {
    pub fn new(max_fee_ceiling_gwei: f64, default_priority_fee_gwei: f64, bump_percent: u32) -> Self {
        Self {
            max_fee_ceiling_wei: gwei_to_wei(max_fee_ceiling_gwei),
            default_priority_fee_wei: gwei_to_wei(default_priority_fee_gwei),
            bump_percent: bump_percent.max(12), // Minimum 12% to guarantee EVM replacement acceptance
        }
    }

    /// Computes EIP-1559 fees based on current block base fee:
    /// max_fee = (2 * base_fee) + max_priority_fee
    pub fn calculate_dynamic_fees(
        &self,
        current_base_fee_wei: u128,
        custom_priority_gwei: Option<f64>,
    ) -> Result<(u128, u128), GasError> {
        let priority_fee = custom_priority_gwei
            .map(gwei_to_wei)
            .unwrap_or(self.default_priority_fee_wei);

        let max_fee = current_base_fee_wei
            .checked_mul(2)
            .and_then(|v| v.checked_add(priority_fee))
            .ok_or(GasError::Overflow)?;

        if max_fee > self.max_fee_ceiling_wei {
            return Err(GasError::ExceedsCeiling {
                calculated_gwei: wei_to_gwei(max_fee),
                ceiling_gwei: wei_to_gwei(self.max_fee_ceiling_wei),
            });
        }

        Ok((max_fee, priority_fee))
    }

    /// Computes replacement transaction gas fees (auto-speedup) bumping by `bump_percent` (e.g. 15%).
    /// Guaranteed to satisfy geth/reth replace-by-fee requirements.
    pub fn calculate_speedup_fees(
        &self,
        prev_max_fee_wei: u128,
        prev_priority_fee_wei: u128,
    ) -> Result<(u128, u128), GasError> {
        let multiplier = 100 + self.bump_percent as u128;

        let new_max_fee = prev_max_fee_wei
            .checked_mul(multiplier)
            .map(|v| v.div_ceil(100)) // round up
            .ok_or(GasError::Overflow)?;

        let new_priority_fee = prev_priority_fee_wei
            .checked_mul(multiplier)
            .map(|v| v.div_ceil(100)) // round up
            .ok_or(GasError::Overflow)?;

        if new_max_fee > self.max_fee_ceiling_wei {
            return Err(GasError::ExceedsCeiling {
                calculated_gwei: wei_to_gwei(new_max_fee),
                ceiling_gwei: wei_to_gwei(self.max_fee_ceiling_wei),
            });
        }

        Ok((new_max_fee, new_priority_fee))
    }
}

pub fn gwei_to_wei(gwei: f64) -> u128 {
    (gwei * 1_000_000_000.0).round() as u128
}

pub fn wei_to_gwei(wei: u128) -> f64 {
    wei as f64 / 1_000_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_speedup_fees_bump() {
        let engine = GasEngine::new(100.0, 2.0, 15);
        let prev_max = 20_000_000_000u128;
        let prev_prio = 2_000_000_000u128;
        let (new_max, new_prio) = engine.calculate_speedup_fees(prev_max, prev_prio).unwrap();
        assert_eq!(new_max, 23_000_000_000);
        assert_eq!(new_prio, 2_300_000_000);
    }

    #[test]
    fn test_ceiling_enforced() {
        let engine = GasEngine::new(25.0, 2.0, 15);
        let res = engine.calculate_speedup_fees(24_000_000_000, 2_000_000_000);
        assert!(res.is_err());
    }
}
