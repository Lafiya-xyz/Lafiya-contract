// Test cases for .semgrep/soroban.yml (run: semgrep --test --config .semgrep/soroban.yml .semgrep/soroban.rs)

impl Contract {
    // ruleid: soroban-storage-write-without-auth
    pub fn set_unguarded(env: Env, v: u32) { env.storage().instance().set(&DataKey::V, &v); }

    pub fn set_guarded(env: Env, admin: Address, v: u32) {
        admin.require_auth();
        // ok: soroban-storage-write-without-auth
        env.storage().instance().set(&DataKey::V, &v);
    }

    pub fn set_guarded_result(env: Env, v: u32) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        // ok: soroban-storage-write-without-auth
        env.storage().persistent().remove(&DataKey::V);
        Ok(())
    }

    pub fn write_then_auth(env: Env, admin: Address) {
        // ruleid: soroban-storage-write-before-auth
        env.storage().instance().set(&DataKey::Paused, &true);
        admin.require_auth();
    }

    pub fn auth_then_write(env: Env, admin: Address) {
        admin.require_auth();
        // ok: soroban-storage-write-before-auth
        env.storage().instance().set(&DataKey::Paused, &true);
    }

    pub fn call_other(env: Env, id: Address) -> Result<u32, Error> {
        let client = RegistryClient::new(&env, &id);
        // ok: soroban-panicking-cross-contract-call
        let a = client.try_count().map_err(|_| Error::Call)?;
        // ruleid: soroban-panicking-cross-contract-call
        let b = client.count();
        Ok(a.unwrap() + b)
    }

    pub fn loop_unbounded(env: Env, items: Vec<Address>) {
        // ruleid: soroban-unbounded-param-loop
        for item in items.iter() {
            env.events().publish((item,), ());
        }
    }

    pub fn loop_bounded(env: Env, items: Vec<Address>) -> Result<(), Error> {
        if items.len() > MAX_BATCH {
            return Err(Error::BatchTooLarge);
        }
        // ok: soroban-unbounded-param-loop
        for item in items.iter() {
            env.events().publish((item,), ());
        }
        Ok(())
    }

    fn admin_bad(env: &Env, fallback: Address) -> Address {
        // ruleid: soroban-admin-read-unwrap-or
        env.storage().instance().get(&DataKey::Admin).unwrap_or(fallback)
    }

    fn admin_good(env: &Env) -> Result<Address, Error> {
        // ok: soroban-admin-read-unwrap-or
        env.storage().instance().get(&DataKey::Admin).ok_or(Error::NotInitialized)
    }
}

impl Multisig {
    pub fn __constructor(env: Env, signers: Vec<BytesN<32>>) {
        // ok: soroban-unbounded-param-loop
        for s in signers.iter() {
            // ok: soroban-storage-write-without-auth
            env.storage().instance().set(&DataKey::Signer(s), &());
        }
    }
}
