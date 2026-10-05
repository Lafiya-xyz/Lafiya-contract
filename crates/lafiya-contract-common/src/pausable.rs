//! Pause control: pause/unpause/is_paused operations.

use soroban_sdk::{Address, Env};

/// Error types for pause operations.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PauseError {
    ContractPaused,
}

/// Trait for providing contract-specific pause storage key.
pub trait PausableKeys {
    type PauseKey;
    fn pause_key() -> Self::PauseKey;
}

/// Pausable module: provides pause/unpause/is_paused operations.
pub struct Pausable;

impl Pausable {
    /// Check if the contract is currently paused.
    pub fn is_paused<K: PausableKeys>(env: &Env) -> bool {
        env.storage()
            .instance()
            .get::<K::PauseKey, bool>(&K::pause_key())
            .unwrap_or(false)
    }

    /// Pause the contract. Requires admin authorization (caller's responsibility).
    pub fn pause<K: PausableKeys>(env: &Env) -> Result<(), PauseError> {
        env.storage()
            .instance()
            .set(&K::pause_key(), &true);
        Ok(())
    }

    /// Unpause the contract. Requires admin authorization (caller's responsibility).
    pub fn unpause<K: PausableKeys>(env: &Env) -> Result<(), PauseError> {
        env.storage()
            .instance()
            .set(&K::pause_key(), &false);
        Ok(())
    }

    /// Require that the contract is not paused. Returns `PauseError::ContractPaused` if paused.
    pub fn require_not_paused<K: PausableKeys>(env: &Env) -> Result<(), PauseError> {
        if Self::is_paused::<K>(env) {
            return Err(PauseError::ContractPaused);
        }
        Ok(())
    }
}
