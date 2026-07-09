use super::*;
use soroban_sdk::Env;

#[test]
fn ping_returns_true() {
    let env = Env::default();
    let contract_id = env.register(AttesterRegistry, ());
    let client = AttesterRegistryClient::new(&env, &contract_id);

    assert!(client.ping());
}
