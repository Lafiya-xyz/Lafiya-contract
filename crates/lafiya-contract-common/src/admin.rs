//! Admin management: two-step admin transfer with auth.

use soroban_sdk::{Address, Env};

/// Error types for admin operations.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AdminError {
    NotInitialized,
    NoPendingTransfer,
}

/// Trait for providing contract-specific storage keys.
/// Implemented by each contract to supply Admin and PendingAdmin keys.
pub trait AdminKeys {
    type AdminKey;
    type PendingAdminKey;

    fn admin_key() -> Self::AdminKey;
    fn pending_admin_key() -> Self::PendingAdminKey;
}

/// Admin module: provides two-step admin transfer.
pub struct Admin;

impl Admin {
    /// Retrieve the current admin. Returns `AdminError::NotInitialized` if not set.
    pub fn get<K: AdminKeys>(env: &Env) -> Result<Address, AdminError> {
        env.storage()
            .instance()
            .get(&K::admin_key())
            .ok_or(AdminError::NotInitialized)
    }

    /// Propose a new admin. Requires current admin's authorization.
    /// Calling this a second time before `accept` overwrites any pending proposal.
    pub fn propose<K: AdminKeys>(env: &Env, new_admin: Address) -> Result<(), AdminError> {
        let current_admin = Self::get::<K>(env)?;
        current_admin.require_auth();
        env.storage()
            .instance()
            .set(&K::pending_admin_key(), &new_admin);
        Ok(())
    }

    /// Accept the proposed admin transfer. Requires pending admin's authorization.
    pub fn accept<K: AdminKeys>(env: &Env) -> Result<Address, AdminError> {
        let previous_admin = Self::get::<K>(env)?;
        let pending_admin: Address = env
            .storage()
            .instance()
            .get(&K::pending_admin_key())
            .ok_or(AdminError::NoPendingTransfer)?;

        pending_admin.require_auth();

        env.storage()
            .instance()
            .set(&K::admin_key(), &pending_admin);
        env.storage().instance().remove(&K::pending_admin_key());

        Ok(previous_admin)
    }

    /// Require that the caller is the current admin.
    pub fn require_auth<K: AdminKeys>(env: &Env) -> Result<(), AdminError> {
        let admin = Self::get::<K>(env)?;
        admin.require_auth();
        Ok(())
    }
}
