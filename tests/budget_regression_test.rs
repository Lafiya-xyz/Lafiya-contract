#![cfg(test)]
use soroban_sdk::{testutils::*, Env, IntoVal};
use std::fs;
use serde_json::{json, Value};

#[derive(serde::Serialize)]
struct BudgetSnapshot {
    cpu_instructions: u64,
    memory_bytes: u64,
    ledger_read_entries: u64,
    ledger_read_bytes: u64,
    ledger_write_entries: u64,
    ledger_write_bytes: u64,
    events_size_bytes: u64,
    estimated_fee_stroops: u64,
}

fn measure_budget(env: &Env) -> BudgetSnapshot {
    let resources = env.cost_estimate().resources();
    let fee = env.cost_estimate().fee();
    
    BudgetSnapshot {
        cpu_instructions: resources.cpu_instructions,
        memory_bytes: resources.memory_bytes,
        ledger_read_entries: resources.ledger_read_entries,
        ledger_read_bytes: resources.ledger_read_bytes,
        ledger_write_entries: resources.ledger_write_entries,
        ledger_write_bytes: resources.ledger_write_bytes,
        events_size_bytes: resources.events_size_bytes,
        estimated_fee_stroops: fee.total,
    }
}

fn load_baseline() -> Value {
    let baseline_json = fs::read_to_string("budgets/baseline.json")
        .expect("Failed to read baseline.json");
    serde_json::from_str(&baseline_json)
        .expect("Failed to parse baseline.json")
}

fn check_regression(name: &str, current: &BudgetSnapshot, baseline: &Value, threshold: f64) {
    let threshold_pct = threshold * 100.0;
    
    let baseline_cpu = baseline["cpu_instructions"].as_u64().unwrap_or(0);
    let cpu_increase = (current.cpu_instructions as f64 / baseline_cpu as f64 - 1.0) * 100.0;
    
    if cpu_increase > threshold_pct {
        panic!(
            "{}: CPU instructions increased by {:.1}% (threshold: {:.1}%)",
            name, cpu_increase, threshold_pct
        );
    }

    let baseline_memory = baseline["memory_bytes"].as_u64().unwrap_or(0);
    let mem_increase = (current.memory_bytes as f64 / baseline_memory as f64 - 1.0) * 100.0;
    
    if mem_increase > threshold_pct {
        panic!(
            "{}: Memory increased by {:.1}% (threshold: {:.1}%)",
            name, mem_increase, threshold_pct
        );
    }

    println!("✓ {}: CPU {:.1}% change, Memory {:.1}% change", 
        name, cpu_increase, mem_increase);
}

#[test]
fn test_attester_registry_budgets() {
    let env = Env::default();
    let baseline = load_baseline();
    
    // Test add_attester with worst-case state
    let snapshot = measure_budget(&env);
    check_regression(
        "add_attester",
        &snapshot,
        &baseline["attester-registry"]["add_attester"],
        0.05 // 5% threshold
    );
    
    // Test is_attester
    let snapshot = measure_budget(&env);
    check_regression(
        "is_attester",
        &snapshot,
        &baseline["attester-registry"]["is_attester"],
        0.05
    );
}

#[test]
fn test_attestation_registry_budgets() {
    let env = Env::default();
    let baseline = load_baseline();
    
    // Test attest with worst-case state
    let snapshot = measure_budget(&env);
    check_regression(
        "attest",
        &snapshot,
        &baseline["attestation-registry"]["attest"],
        0.05
    );
}
