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
        let max_fee_ceiling_wei = gwei_to_wei(max_fee_ceiling_gwei);
        // Priority fee must never exceed max fee ceiling (EIP-1559 ErrTipAboveFeeCap protection).
        let default_priority_fee_wei = gwei_to_wei(default_priority_fee_gwei).min(max_fee_ceiling_wei);
        Self {
            max_fee_ceiling_wei,
            default_priority_fee_wei,
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
            .unwrap_or(self.default_priority_fee_wei)
            .min(self.max_fee_ceiling_wei);

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

    /// Computes EIP-1559 fees with momentum / acceleration protection:
    /// If consecutive base fees indicate an upward surge, projects the base fee `blocks_ahead` into the future
    /// (compounding up to 12.5% maximum EVM increase per block per EIP-1559) so pre-signed transactions
    /// are not priced out during rapid gas spikes.
    pub fn calculate_dynamic_fees_with_acceleration(
        &self,
        current_base_fee_wei: u128,
        prev_base_fee_wei: Option<u128>,
        blocks_ahead: u32,
        custom_priority_gwei: Option<f64>,
    ) -> Result<(u128, u128), GasError> {
        let mut projected_base_fee = current_base_fee_wei;

        if let Some(prev) = prev_base_fee_wei {
            if current_base_fee_wei > prev && prev > 0 {
                // Observed rate of change in basis points (100 bps = 1%)
                // In EIP-1559, max increase per block is 12.5% (1250 bps).
                let delta = current_base_fee_wei.saturating_sub(prev);
                let rate_bps = (delta.saturating_mul(10_000) / prev).min(1250);

                for _ in 0..blocks_ahead {
                    let increase = (projected_base_fee.saturating_mul(rate_bps)).div_ceil(10_000);
                    projected_base_fee = projected_base_fee.saturating_add(increase);
                }
            }
        }

        self.calculate_dynamic_fees(projected_base_fee, custom_priority_gwei)
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
            .ok_or(GasError::Overflow)?
            .min(new_max_fee);

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
    if !gwei.is_finite() || gwei <= 0.0 {
        return 0;
    }
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

    #[test]
    fn test_priority_fee_never_exceeds_max_fee() {
        // Low-gas L2 scenario: user sets max_fee_gwei = 0.2 while default_priority_fee = 1.0
        let engine = GasEngine::new(0.2, 1.0, 15);
        let (max_fee, prio_fee) = engine.calculate_dynamic_fees(0, None).unwrap();
        assert!(prio_fee <= max_fee, "priority fee must never exceed max fee");
        assert_eq!(max_fee, 200_000_000);
        assert_eq!(prio_fee, 200_000_000);
    }

    #[test]
    fn test_dynamic_fees_with_acceleration() {
        let engine = GasEngine::new(100.0, 2.0, 15);
        let base_fee_prev = 10_000_000_000u128; // 10 Gwei
        let base_fee_curr = 11_000_000_000u128; // 11 Gwei (+10%)

        // No acceleration: (2 * 11) + 2 = 24 Gwei
        let (max_no_accel, _) = engine.calculate_dynamic_fees(base_fee_curr, None).unwrap();
        assert_eq!(max_no_accel, 24_000_000_000);

        // With 1 block acceleration (+10% on 11 Gwei = 12.1 Gwei -> (2 * 12.1) + 2 = 26.2 Gwei)
        let (max_accel, prio) = engine.calculate_dynamic_fees_with_acceleration(
            base_fee_curr,
            Some(base_fee_prev),
            1,
            None,
        ).unwrap();
        assert_eq!(prio, 2_000_000_000);
        assert!(max_accel > max_no_accel);
        assert_eq!(max_accel, 26_200_000_000);
    }
}
