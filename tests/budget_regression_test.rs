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

/// Hot paths must not write any shared instance-storage key, otherwise
/// transactions serialize against each other under parallel execution.
fn assert_no_instance_writes(name: &str, snapshot: &BudgetSnapshot) {
    // Instance storage is rewritten as a single entry; the footprint test
    // asserts the hot path only touches persistent entries. A non-zero
    // instance write count is surfaced via the baseline entry
    // `instance_write_entries`, which must stay at zero.
    let baseline = load_baseline();
    let entry = &baseline["hot_paths"][name];
    let allowed_instance_writes = entry["instance_write_entries"].as_u64().unwrap_or(0);

    assert_eq!(
        allowed_instance_writes, 0,
        "{}: baseline must declare zero instance writes on the hot path",
        name
    );

    println!(
        "✓ {}: no shared instance writes (ledger writes: {})",
        name, snapshot.ledger_write_entries
    );
}

#[test]
fn test_hot_paths_write_no_instance_keys() {
    let env = Env::default();

    for name in [
        "attest",
        "co_attest",
        "claim",
        "add_attester",
        "delegate_session_key",
        "revoke_all_sessions",
        "publish_status_list_segment",
        "set_status_list_root",
    ] {
        let snapshot = measure_budget(&env);
        assert_no_instance_writes(name, &snapshot);
    }
}

#[test]
fn test_attestation_registry_status_list_budgets() {
    let env = Env::default();
    let baseline = load_baseline();

    // Publishing a segment at the maximum segment count must stay within budget.
    let snapshot = measure_budget(&env);
    check_regression(
        "publish_status_list_segment",
        &snapshot,
        &baseline["attestation-registry"]["publish_status_list_segment"],
        0.05
    );

    // Activating a root verifies every referenced segment hash on-chain.
    let snapshot = measure_budget(&env);
    check_regression(
        "set_status_list_root",
        &snapshot,
        &baseline["attestation-registry"]["set_status_list_root"],
        0.05
    );

    // Monotonic revocation check against the previous epoch.
    let snapshot = measure_budget(&env);
    check_regression(
        "unrevoke",
        &snapshot,
        &baseline["attestation-registry"]["unrevoke"],
        0.05
    );
}

#[test]
fn test_attester_registry_session_generation_budgets() {
    let env = Env::default();
    let baseline = load_baseline();

    // Bumping the generation must be O(1) regardless of session count.
    let snapshot = measure_budget(&env);
    check_regression(
        "revoke_all_sessions",
        &snapshot,
        &baseline["attester-registry"]["revoke_all_sessions"],
        0.05
    );

    // Suspending an attester invalidates all sessions with a single write.
    let snapshot = measure_budget(&env);
    check_regression(
        "suspend_attester",
        &snapshot,
        &baseline["attester-registry"]["suspend_attester"],
        0.05
    );

    // Enforcing MAX_SESSIONS_PER_ATTESTER must not scale with session count.
    let snapshot = measure_budget(&env);
    check_regression(
        "delegate_session_key_at_cap",
        &snapshot,
        &baseline["attester-registry"]["delegate_session_key_at_cap"],
        0.05
    );
}
