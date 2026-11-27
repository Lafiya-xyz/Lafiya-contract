struct S; struct P; struct E;
impl E { fn storage(&self) -> S { S } }
impl S { fn persistent(&self) -> P { P } }
impl P { fn set(&self, _k: u32, _v: u32) {} fn extend_ttl(&self, _k: u32, _a: u32, _b: u32) {} }

fn bad(e: &E) {
    e.storage().persistent().set(1, 2); //~ persistent_set_without_ttl
}
fn good(e: &E) {
    e.storage().persistent().set(1, 2);
    e.storage().persistent().extend_ttl(1, 10, 20);
}
#[allow(persistent_set_without_ttl, reason = "TTL is extended by the caller")]
fn allowed(e: &E) {
    e.storage().persistent().set(1, 2);
}

// --- UI fixture for `non_try_cross_contract_call` ---
//
// Models a cross-contract client: `RegistryClient` is a generated Soroban
// client type whose methods panic (trap) on transport or contract failure.
// The lint flags direct (non-`try_`) calls on such clients in contract code.
mod registry_client {
    pub struct RegistryClient;
    impl RegistryClient {
        pub fn is_attester(&self, _attester: &u32) -> bool { true }
        pub fn try_is_attester(&self, _attester: &u32) -> Result<bool, u32> { Ok(true) }
    }
}

fn bad_cross_contract_call(client: &registry_client::RegistryClient) {
    let _ = client.is_attester(&1); //~ non_try_cross_contract_call
}

fn good_cross_contract_call(client: &registry_client::RegistryClient) {
    let _ = client.try_is_attester(&1);
}

#[allow(non_try_cross_contract_call, reason = "call is infallible by construction")]
fn allowed_cross_contract_call(client: &registry_client::RegistryClient) {
    let _ = client.is_attester(&1);
}

fn main() {}