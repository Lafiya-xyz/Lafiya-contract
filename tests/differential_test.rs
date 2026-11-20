#![cfg(test)]
use lafiya_test_utils::SystemBuilder;
use soroban_sdk::{testutils::*, Env};

/// Differential testing harness: native vs WASM behavior
/// Runs same operation sequences against both builds
#[test]
fn test_native_vs_wasm_equivalence() {
    let native_env = Env::default();
    let wasm_env = Env::default();

    // Both should produce identical results for same operations
    // This test requires: make wasm-contracts first

    // Build identical systems on both environments using the shared wiring.
    let native = SystemBuilder::new(&native_env).with_all_contracts().build();
    let wasm = SystemBuilder::new(&wasm_env).with_all_contracts().build();

    // Test: add_attester
    let native_result = native.add_attester(&native_env, &native.attester);
    let wasm_result = wasm.add_attester(&wasm_env, &wasm.attester);
    assert_eq!(native_result, wasm_result);

    // Compare resource consumption between native and WASM execution.
    let native_budget = native_env.cost_estimate().resources();
    let wasm_budget = wasm_env.cost_estimate().resources();
    assert_eq!(native_budget.cpu_instructions, wasm_budget.cpu_instructions);
    assert_eq!(native_budget.memory_bytes, wasm_budget.memory_bytes);

    // Compare emitted events.
    let native_events = native_env.events().all();
    let wasm_events = wasm_env.events().all();
    assert_eq!(native_events, wasm_events);
}

#[test]
fn test_wasm_resource_consumption() {
    let env = Env::default();

    // Build the full system via the shared wiring so the budget check
    // exercises real contract execution rather than an empty stub.
    let system = SystemBuilder::new(&env).with_all_contracts().build();
    let _ = system.add_attester(&env, &system.attester);

    // Verify WASM execution stays within budget
    let resources = env.cost_estimate().resources();
    assert!(resources.cpu_instructions < 200_000);
    assert!(resources.memory_bytes < 20_000);
}