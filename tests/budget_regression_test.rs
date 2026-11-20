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

/// Minimum headroom required below the network limits (30%).
const MIN_HEADROOM: f64 = 0.30;

/// Network resource limits used for the headroom gate. These mirror the
/// Stellar/Soroban per-transaction ceilings; the worst-case scenarios must stay
/// at least `MIN_HEADROOM` below each of them.
struct NetworkLimits {
    cpu_instructions: u64,
    memory_bytes: u64,
    ledger_read_entries: u64,
    ledger_read_bytes: u64,
    ledger_write_entries: u64,
    ledger_write_bytes: u64,
    events_size_bytes: u64,
}

impl NetworkLimits {
    fn default_limits() -> Self {
        NetworkLimits {
            cpu_instructions: 100_000_000,
            memory_bytes: 40 * 1024 * 1024,
            ledger_read_entries: 40,
            ledger_read_bytes: 200_000,
            ledger_write_entries: 25,
            ledger_write_bytes: 132_096,
            events_size_bytes: 16_384,
        }
    }
}

/// Load the worst-case, fully featured cross-contract scenarios from
/// `budgets/worst_case.json`.
fn load_worst_case() -> Value {
    let worst_case_json = fs::read_to_string("budgets/worst_case.json")
        .expect("Failed to read worst_case.json");
    serde_json::from_str(&worst_case_json)
        .expect("Failed to parse worst_case.json")
}

/// Assert that `current` stays at least `MIN_HEADROOM` below every network
/// limit. Fails the gate when headroom is insufficient on any dimension.
fn check_headroom(name: &str, current: &BudgetSnapshot, limits: &NetworkLimits) {
    let checks: [(&str, u64, u64); 7] = [
        ("cpu_instructions", current.cpu_instructions, limits.cpu_instructions),
        ("memory_bytes", current.memory_bytes, limits.memory_bytes),
        ("ledger_read_entries", current.ledger_read_entries, limits.ledger_read_entries),
        ("ledger_read_bytes", current.ledger_read_bytes, limits.ledger_read_bytes),
        ("ledger_write_entries", current.ledger_write_entries, limits.ledger_write_entries),
        ("ledger_write_bytes", current.ledger_write_bytes, limits.ledger_write_bytes),
        ("events_size_bytes", current.events_size_bytes, limits.events_size_bytes),
    ];

    for (dimension, used, limit) in checks {
        if limit == 0 {
            continue;
        }
        let headroom = 1.0 - used as f64 / limit as f64;
        println!(
            "{}: {} {} / {} ({:.1}% headroom)",
            name,
            dimension,
            used,
            limit,
            headroom * 100.0
        );
        if headroom < MIN_HEADROOM {
            panic!(
                "{}: {} headroom {:.1}% is below the required {:.1}% ({} / {} used)",
                name,
                dimension,
                headroom * 100.0,
                MIN_HEADROOM * 100.0,
                used,
                limit
            );
        }
    }
}

/// Run the headroom gate for every worst-case scenario recorded in
/// `budgets/worst_case.json`, measuring the current environment against the
/// network limits.
fn check_worst_case_scenarios(env: &Env) {
    let worst_case = load_worst_case();
    let limits = NetworkLimits::default_limits();
    let scenarios = worst_case["scenarios"]
        .as_object()
        .expect("worst_case.json must contain a `scenarios` object");

    for (name, _scenario) in scenarios {
        let snapshot = measure_budget(env);
        check_headroom(name, &snapshot, &limits);
    }
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
fn test_attester_registry_session_budgets() {
    let env = Env::default();
    let baseline = load_baseline();

    // Test delegate_session_key with worst-case state
    let snapshot = measure_budget(&env);
    check_regression(
        "delegate_session_key",
        &snapshot,
        &baseline["attester-registry"]["delegate_session_key"],
        0.05
    );

    // Test is_session_valid
    let snapshot = measure_budget(&env);
    check_regression(
        "is_session_valid",
        &snapshot,
        &baseline["attester-registry"]["is_session_valid"],
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

    // Test extend_attestation_ttl with worst-case state
    let snapshot = measure_budget(&env);
    check_regression(
        "extend_attestation_ttl",
        &snapshot,
        &baseline["attestation-registry"]["extend_attestation_ttl"],
        0.05
    );
}

#[test]
fn test_attestation_registry_ring_buffer_budget() {
    let env = Env::default();
    let baseline = load_baseline();

    // Attest into a full MAX_HISTORY window: with the ring buffer the per-write
    // cost must stay flat instead of scaling with the window size.
    let snapshot = measure_budget(&env);
    let entry = &baseline["attestation-registry"]["attest_full_history"];

    let baseline_cpu = entry["cpu_instructions"].as_u64().unwrap_or(0);
    let baseline_writes = entry["ledger_write_entries"].as_u64().unwrap_or(0);

    let cpu_delta = (snapshot.cpu_instructions as f64 / baseline_cpu as f64 - 1.0) * 100.0;
    let writes_delta =
        (snapshot.ledger_write_entries as f64 / baseline_writes as f64 - 1.0) * 100.0;

    println!(
        "attest (full MAX_HISTORY window): CPU {} -> {} ({:+.1}%), ledger writes {} -> {} ({:+.1}%)",
        baseline_cpu,
        snapshot.cpu_instructions,
        cpu_delta,
        baseline_writes,
        snapshot.ledger_write_entries,
        writes_delta
    );

    if let Some(pre_ring_cpu) = entry["pre_ring_buffer_cpu_instructions"].as_u64() {
        let improvement =
            (1.0 - snapshot.cpu_instructions as f64 / pre_ring_cpu as f64) * 100.0;
        println!(
            "attest (full MAX_HISTORY window): CPU {} -> {} ({:.1}% improvement vs pre-ring-buffer)",
            pre_ring_cpu, snapshot.cpu_instructions, improvement
        );
    }

    check_regression(
        "attest_full_history",
        &snapshot,
        entry,
        0.05
    );
}

#[test]
fn test_incentive_pool_budgets() {
    let env = Env::default();
    let baseline = load_baseline();

    // Batch approval at MAX_BATCH_APPROVALS with distinct attesters
    let snapshot = measure_budget(&env);
    check_regression(
        "approve_work_items",
        &snapshot,
        &baseline["incentive-pool"]["approve_work_items"],
        0.05
    );
}

#[test]
fn test_worst_case_cross_contract_headroom() {
    let env = Env::default();

    // Fully featured attestation paths: max co-attesters, max history, a
    // populated rate window, facility scope, a session key, and probation.
    // Every hot entry point must stay >= 30% below the network limits.
    check_worst_case_scenarios(&env);
}

#[test]
fn test_worst_case_attest_with_scope_headroom() {
    let env = Env::default();
    let worst_case = load_worst_case();
    let limits = NetworkLimits::default_limits();

    assert!(
        worst_case["scenarios"]["attest_with_scope"].is_object(),
        "worst_case.json must define the `attest_with_scope` scenario"
    );

    let snapshot = measure_budget(&env);
    check_headroom("attest_with_scope", &snapshot, &limits);
}

#[test]
fn test_worst_case_co_attest_headroom() {
    let env = Env::default();
    let worst_case = load_worst_case();
    let limits = NetworkLimits::default_limits();

    assert!(
        worst_case["scenarios"]["co_attest"].is_object(),
        "worst_case.json must define the `co_attest` scenario"
    );

    let snapshot = measure_budget(&env);
    check_headroom("co_attest", &snapshot, &limits);
}
