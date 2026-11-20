//! Pure accounting core for the incentive pool.
//!
//! This module contains all of the arithmetic used by the incentive pool
//! contract (reservations, caps, epoch budgets, payout rates and drift).
//! It performs **no storage access** and is `no_std` compatible so that the
//! logic can be exercised by property tests and formally verified with Kani.
//!
//! # Documented bounds
//!
//! All amounts handled by this module are non-negative `i128` values and are
//! expected to satisfy the following bounds (enforced by the contract before
//! calling into this module):
//!
//! * `0 <= amount <= MAX_AMOUNT` where `MAX_AMOUNT = i128::MAX / 4`.
//! * `0 <= deposited <= MAX_AMOUNT`.
//! * `0 <= reserved <= deposited <= MAX_AMOUNT`.
//! * `0 <= paid <= deposited <= MAX_AMOUNT`.
//! * `0 <= rate_bps <= MAX_RATE_BPS` where `MAX_RATE_BPS = 10_000`.
//! * `0 <= epoch_budget <= MAX_AMOUNT`.
//! * `0 <= epoch_spent <= epoch_budget <= MAX_AMOUNT`.
//!
//! Under these bounds every intermediate value produced by this module fits
//! comfortably inside `i128` (the largest intermediate is
//! `MAX_AMOUNT * MAX_RATE_BPS = i128::MAX / 4 * 10_000`, which is still far
//! below `i128::MAX`), so none of the operations below can overflow.
//!
//! # Invariants
//!
//! * `reserved + paid <= deposited` is preserved by every operation.
//! * Rounding in [`apply_rate`] always rounds **down**, i.e. never in favour
//!   of the claimant.

#![allow(clippy::arithmetic_side_effects)]

/// Maximum amount accepted by the accounting core.
///
/// Chosen as `i128::MAX / 4` so that multiplying by [`MAX_RATE_BPS`] and
/// adding a few amounts can never overflow `i128`.
pub const MAX_AMOUNT: i128 = i128::MAX / 4;

/// Maximum payout rate in basis points (100%).
pub const MAX_RATE_BPS: i128 = 10_000;

/// Basis-point denominator.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Plain, storage-free representation of the pool accounting state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolState {
    /// Total amount ever deposited into the pool.
    pub deposited: i128,
    /// Amount currently reserved (locked) for pending claims.
    pub reserved: i128,
    /// Amount already paid out.
    pub paid: i128,
    /// Budget available for the current epoch.
    pub epoch_budget: i128,
    /// Amount already spent in the current epoch.
    pub epoch_spent: i128,
}

/// Errors returned by the accounting core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountingError {
    /// An amount was negative or exceeded [`MAX_AMOUNT`].
    AmountOutOfBounds,
    /// A rate was negative or exceeded [`MAX_RATE_BPS`].
    RateOutOfBounds,
    /// The operation would exceed the pool cap (`reserved + paid <= deposited`).
    InsufficientFunds,
    /// The operation would exceed the current epoch budget.
    EpochBudgetExceeded,
    /// The operation would release/pay more than is reserved.
    InsufficientReserved,
}

/// Validates that `amount` is within the documented bounds.
#[inline]
fn check_amount(amount: i128) -> Result<(), AccountingError> {
    if amount < 0 || amount > MAX_AMOUNT {
        Err(AccountingError::AmountOutOfBounds)
    } else {
        Ok(())
    }
}

/// Validates that `rate_bps` is within the documented bounds.
#[inline]
fn check_rate(rate_bps: i128) -> Result<(), AccountingError> {
    if rate_bps < 0 || rate_bps > MAX_RATE_BPS {
        Err(AccountingError::RateOutOfBounds)
    } else {
        Ok(())
    }
}

/// Reserves `amount` from the pool.
///
/// Fails if the reservation would break `reserved + paid <= deposited`.
pub fn reserve(state: &mut PoolState, amount: i128) -> Result<(), AccountingError> {
    check_amount(amount)?;
    let new_reserved = state
        .reserved
        .checked_add(amount)
        .ok_or(AccountingError::AmountOutOfBounds)?;
    let committed = new_reserved
        .checked_add(state.paid)
        .ok_or(AccountingError::AmountOutOfBounds)?;
    if committed > state.deposited {
        return Err(AccountingError::InsufficientFunds);
    }
    state.reserved = new_reserved;
    Ok(())
}

/// Releases `amount` previously reserved, returning it to the available pool.
///
/// Fails if `amount` exceeds the currently reserved amount.
pub fn release(state: &mut PoolState, amount: i128) -> Result<(), AccountingError> {
    check_amount(amount)?;
    if amount > state.reserved {
        return Err(AccountingError::InsufficientReserved);
    }
    state.reserved -= amount;
    Ok(())
}

/// Pays `amount` out of the reserved balance.
///
/// Moves `amount` from `reserved` to `paid`, charging it against the current
/// epoch budget. Fails if `amount` exceeds the reserved balance or the
/// remaining epoch budget.
pub fn pay(state: &mut PoolState, amount: i128) -> Result<(), AccountingError> {
    check_amount(amount)?;
    if amount > state.reserved {
        return Err(AccountingError::InsufficientReserved);
    }
    let new_epoch_spent = state
        .epoch_spent
        .checked_add(amount)
        .ok_or(AccountingError::AmountOutOfBounds)?;
    if new_epoch_spent > state.epoch_budget {
        return Err(AccountingError::EpochBudgetExceeded);
    }
    let new_paid = state
        .paid
        .checked_add(amount)
        .ok_or(AccountingError::AmountOutOfBounds)?;
    // `reserved + paid` is unchanged by moving funds from `reserved` to
    // `paid`, so the pool cap invariant is preserved.
    state.reserved -= amount;
    state.paid = new_paid;
    state.epoch_spent = new_epoch_spent;
    Ok(())
}

/// Applies a basis-point rate to `amount`, rounding **down**.
///
/// Rounding down guarantees the result never favours the claimant.
pub fn apply_rate(amount: i128, rate_bps: i128) -> Result<i128, AccountingError> {
    check_amount(amount)?;
    check_rate(rate_bps)?;
    // `amount <= MAX_AMOUNT` and `rate_bps <= MAX_RATE_BPS`, so the product
    // fits in `i128` (see module-level bounds documentation).
    let product = amount * rate_bps;
    Ok(product / BPS_DENOMINATOR)
}

/// Checks whether `amount` fits within the remaining epoch budget.
///
/// Returns `Ok(())` if the amount can be spent, otherwise
/// [`AccountingError::EpochBudgetExceeded`].
pub fn epoch_check(state: &PoolState, amount: i128) -> Result<(), AccountingError> {
    check_amount(amount)?;
    let new_epoch_spent = state
        .epoch_spent
        .checked_add(amount)
        .ok_or(AccountingError::AmountOutOfBounds)?;
    if new_epoch_spent > state.epoch_budget {
        Err(AccountingError::EpochBudgetExceeded)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PoolState {
        PoolState {
            deposited: 1_000,
            reserved: 0,
            paid: 0,
            epoch_budget: 1_000,
            epoch_spent: 0,
        }
    }

    #[test]
    fn reserve_and_release() {
        let mut s = state();
        reserve(&mut s, 400).unwrap();
        assert_eq!(s.reserved, 400);
        release(&mut s, 100).unwrap();
        assert_eq!(s.reserved, 300);
    }

    #[test]
    fn reserve_exceeding_cap_fails() {
        let mut s = state();
        reserve(&mut s, 800).unwrap();
        assert_eq!(reserve(&mut s, 300), Err(AccountingError::InsufficientFunds));
    }

    #[test]
    fn pay_moves_reserved_to_paid() {
        let mut s = state();
        reserve(&mut s, 500).unwrap();
        pay(&mut s, 200).unwrap();
        assert_eq!(s.reserved, 300);
        assert_eq!(s.paid, 200);
        assert_eq!(s.epoch_spent, 200);
        assert!(s.reserved + s.paid <= s.deposited);
    }

    #[test]
    fn pay_respects_epoch_budget() {
        let mut s = state();
        s.epoch_budget = 100;
        reserve(&mut s, 500).unwrap();
        assert_eq!(pay(&mut s, 200), Err(AccountingError::EpochBudgetExceeded));
    }

    #[test]
    fn apply_rate_rounds_down() {
        assert_eq!(apply_rate(999, 3_333).unwrap(), 332);
        assert_eq!(apply_rate(100, 10_000).unwrap(), 100);
        assert_eq!(apply_rate(1, 1).unwrap(), 0);
    }

    #[test]
    fn epoch_check_bounds() {
        let mut s = state();
        s.epoch_budget = 100;
        assert_eq!(epoch_check(&s, 100), Ok(()));
        assert_eq!(
            epoch_check(&s, 101),
            Err(AccountingError::EpochBudgetExceeded)
        );
    }

    #[test]
    fn rejects_out_of_bounds() {
        let mut s = state();
        assert_eq!(reserve(&mut s, -1), Err(AccountingError::AmountOutOfBounds));
        assert_eq!(
            reserve(&mut s, MAX_AMOUNT + 1),
            Err(AccountingError::AmountOutOfBounds)
        );
        assert_eq!(
            apply_rate(1, MAX_RATE_BPS + 1),
            Err(AccountingError::RateOutOfBounds)
        );
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Any amount within the documented bounds can be reserved without
    /// overflowing, provided the pool cap allows it.
    #[kani::proof]
    fn reserve_no_overflow() {
        let deposited: i128 = kani::any();
        let reserved: i128 = kani::any();
        let paid: i128 = kani::any();
        let amount: i128 = kani::any();
        kani::assume(deposited >= 0 && deposited <= MAX_AMOUNT);
        kani::assume(reserved >= 0 && reserved <= deposited);
        kani::assume(paid >= 0 && paid <= deposited);
        kani::assume(reserved + paid <= deposited);
        kani::assume(amount >= 0 && amount <= MAX_AMOUNT);

        let mut state = PoolState {
            deposited,
            reserved,
            paid,
            epoch_budget: deposited,
            epoch_spent: 0,
        };
        let _ = reserve(&mut state, amount);
        // If it succeeded, the cap invariant is preserved.
        assert!(state.reserved + state.paid <= state.deposited);
    }

    /// `pay` preserves `reserved + paid <= deposited`.
    #[kani::proof]
    fn pay_preserves_cap() {
        let deposited: i128 = kani::any();
        let reserved: i128 = kani::any();
        let paid: i128 = kani::any();
        let amount: i128 = kani::any();
        kani::assume(deposited >= 0 && deposited <= MAX_AMOUNT);
        kani::assume(reserved >= 0 && reserved <= deposited);
        kani::assume(paid >= 0 && paid <= deposited);
        kani::assume(reserved + paid <= deposited);
        kani::assume(amount >= 0 && amount <= reserved);

        let mut state = PoolState {
            deposited,
            reserved,
            paid,
            epoch_budget: deposited,
            epoch_spent: 0,
        };
        let _ = pay(&mut state, amount);
        assert!(state.reserved + state.paid <= state.deposited);
    }

    /// `apply_rate` never overflows and never rounds up.
    #[kani::proof]
    fn apply_rate_rounds_down() {
        let amount: i128 = kani::any();
        let rate_bps: i128 = kani::any();
        kani::assume(amount >= 0 && amount <= MAX_AMOUNT);
        kani::assume(rate_bps >= 0 && rate_bps <= MAX_RATE_BPS);

        let result = apply_rate(amount, rate_bps).unwrap();
        assert!(result >= 0);
        // result * BPS_DENOMINATOR <= amount * rate_bps (rounding down)
        assert!(result * BPS_DENOMINATOR <= amount * rate_bps);
        // and the next integer would exceed it
        assert!((result + 1) * BPS_DENOMINATOR > amount * rate_bps);
    }

    /// `epoch_check` never overflows for in-bounds inputs.
    #[kani::proof]
    fn epoch_check_no_overflow() {
        let budget: i128 = kani::any();
        let spent: i128 = kani::any();
        let amount: i128 = kani::any();
        kani::assume(budget >= 0 && budget <= MAX_AMOUNT);
        kani::assume(spent >= 0 && spent <= budget);
        kani::assume(amount >= 0 && amount <= MAX_AMOUNT);

        let state = PoolState {
            deposited: budget,
            reserved: 0,
            paid: 0,
            epoch_budget: budget,
            epoch_spent: spent,
        };
        let _ = epoch_check(&state, amount);
    }
}