#![no_std]

use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct AttestationRegistry;

#[contractimpl]
impl AttestationRegistry {
    pub fn ping(_env: Env) -> bool {
        true
    }
}

#[cfg(test)]
mod test;
