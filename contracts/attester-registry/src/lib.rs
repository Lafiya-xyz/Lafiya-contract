#![no_std]

use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct AttesterRegistry;

#[contractimpl]
impl AttesterRegistry {
    pub fn ping(_env: Env) -> bool {
        true
    }
}

#[cfg(test)]
mod test;
