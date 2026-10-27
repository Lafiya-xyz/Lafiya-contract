//! Resource-usage harness for the contract wasm in a build directory.
//!
//! Loads each contract from its `.wasm` bytes, not from the crate, so the
//! harness built from today's tree can measure the wasm of any older commit.
//! That is what `scripts/bench_bisect.py` and the history backfill rely on.
//! Prints one flat JSON object of `contract/scenario/metric -> value`.
//!
//! A scenario that fails (an entry point that did not exist yet, or had a
//! different signature, at the measured commit) is reported under `errors`
//! and its metrics are omitted, rather than failing the whole run.
//!
//! Usage: lafiya-bench [--wasm-dir target/wasm32v1-none/release]

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Parser;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{vec, Address, BytesN, Env, IntoVal, Symbol, Val, Vec};

/// Number of attesters enrolled before measuring the "loaded" scenarios.
const LOADED_ATTESTERS: u32 = 50;

#[derive(Parser, Debug)]
#[command(
    name = "lafiya-bench",
    about = "Measure contract resource usage from built wasm"
)]
struct Cli {
    /// Directory holding the contract `.wasm` files
    #[arg(long, default_value = "target/wasm32v1-none/release")]
    wasm_dir: PathBuf,
}

#[derive(Default)]
struct Report {
    metrics: BTreeMap<String, u64>,
    errors: BTreeMap<String, String>,
}

impl Report {
    /// Invoke `func` and record the resources of that single invocation.
    fn measure(
        &mut self,
        env: &Env,
        key: &str,
        contract: &Address,
        func: &str,
        args: Vec<Val>,
    ) -> bool {
        let result = env.try_invoke_contract::<Val, soroban_sdk::Error>(
            contract,
            &Symbol::new(env, func),
            args,
        );
        if let Err(err) = result {
            self.errors.insert(key.to_string(), format!("{err:?}"));
            return false;
        }
        let r = env.cost_estimate().resources();
        let reads = u64::from(r.disk_read_entries) + u64::from(r.memory_read_entries);
        for (metric, value) in [
            ("cpu_instructions", r.instructions.max(0) as u64),
            ("memory_bytes", r.mem_bytes.max(0) as u64),
            ("ledger_read_entries", reads),
            ("ledger_read_bytes", u64::from(r.disk_read_bytes)),
            ("ledger_write_entries", u64::from(r.write_entries)),
            ("ledger_write_bytes", u64::from(r.write_bytes)),
            ("events_size_bytes", u64::from(r.contract_events_size_bytes)),
        ] {
            self.metrics.insert(format!("{key}/{metric}"), value);
        }
        true
    }

    /// Run setup calls whose cost is not recorded; stop at the first failure.
    fn setup(&mut self, env: &Env, key: &str, calls: &[(&Address, &str, Vec<Val>)]) -> bool {
        for (contract, func, args) in calls {
            let result = env.try_invoke_contract::<Val, soroban_sdk::Error>(
                contract,
                &Symbol::new(env, func),
                args.clone(),
            );
            if let Err(err) = result {
                self.errors
                    .insert(key.to_string(), format!("setup {func}: {err:?}"));
                return false;
            }
        }
        true
    }
}

fn load(dir: &Path, report: &mut Report, contract: &str) -> anyhow::Result<std::vec::Vec<u8>> {
    let path = dir.join(format!("{}.wasm", contract.replace('-', "_")));
    let wasm = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    report
        .metrics
        .insert(format!("{contract}/wasm/size_bytes"), wasm.len() as u64);
    Ok(wasm)
}

/// Grant `admin` the registrar role so it can enrol attesters. Best effort:
/// commits that predate roles let the admin enrol directly.
fn grant_registrar(env: &Env, registry: &Address, admin: &Address) {
    let role: Val = vec![env, Symbol::new(env, "Registrar")].into_val(env);
    let _ = env.try_invoke_contract::<Val, soroban_sdk::Error>(
        registry,
        &Symbol::new(env, "grant_role"),
        vec![env, role, admin.into_val(env)],
    );
}

/// Register a contract from wasm. Contracts with a `__constructor` get `args`
/// at deploy time; older commits that still used `initialize` get the same
/// args through that call instead.
fn deploy(env: &Env, wasm: &[u8], args: Vec<Val>) -> Address {
    if wasm.windows(13).any(|w| w == b"__constructor") {
        return env.register(wasm, args);
    }
    let id = env.register(wasm, ());
    env.invoke_contract::<Val>(&id, &Symbol::new(env, "initialize"), args);
    id
}

fn new_env() -> Env {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.cost_estimate().disable_resource_limits();
    env
}

fn attester_registry(dir: &Path, report: &mut Report) -> anyhow::Result<()> {
    let wasm = load(dir, report, "attester-registry")?;
    let env = new_env();
    let admin = Address::generate(&env);
    let registry = deploy(&env, &wasm, vec![&env, admin.into_val(&env)]);
    grant_registrar(&env, &registry, &admin);
    let first = Address::generate(&env);
    report.measure(
        &env,
        "attester-registry/add_attester",
        &registry,
        "add_attester",
        vec![&env, admin.into_val(&env), first.into_val(&env)],
    );
    report.measure(
        &env,
        "attester-registry/is_attester",
        &registry,
        "is_attester",
        vec![&env, first.into_val(&env)],
    );

    let fill: std::vec::Vec<(&Address, &str, Vec<Val>)> = (1..LOADED_ATTESTERS)
        .map(|_| {
            let args = vec![
                &env,
                admin.into_val(&env),
                Address::generate(&env).into_val(&env),
            ];
            (&registry, "add_attester", args)
        })
        .collect();
    if report.setup(&env, "attester-registry/add_attester_loaded", &fill) {
        let next = Address::generate(&env);
        report.measure(
            &env,
            "attester-registry/add_attester_loaded",
            &registry,
            "add_attester",
            vec![&env, admin.into_val(&env), next.into_val(&env)],
        );
    }
    report.measure(
        &env,
        "attester-registry/remove_attester",
        &registry,
        "remove_attester",
        vec![&env, admin.into_val(&env), first.into_val(&env)],
    );
    Ok(())
}

fn attestation_registry(dir: &Path, report: &mut Report) -> anyhow::Result<()> {
    let attesters = std::fs::read(dir.join("attester_registry.wasm"))?;
    let wasm = load(dir, report, "attestation-registry")?;
    let env = new_env();
    let admin = Address::generate(&env);
    let attester_registry = deploy(&env, &attesters, vec![&env, admin.into_val(&env)]);
    let registry = deploy(
        &env,
        &wasm,
        vec![&env, admin.into_val(&env), attester_registry.into_val(&env)],
    );
    grant_registrar(&env, &attester_registry, &admin);
    let attester = Address::generate(&env);
    let patient = Address::generate(&env);
    let record = BytesN::from_array(&env, &[7; 32]);
    let enrol = vec![&env, admin.into_val(&env), attester.into_val(&env)];
    if !report.setup(
        &env,
        "attestation-registry/attest",
        &[(&attester_registry, "add_attester", enrol)],
    ) {
        return Ok(());
    }
    // Best effort: commits that predate patient consent attest without it.
    let consent = vec![
        &env,
        patient.into_val(&env),
        attester.into_val(&env),
        record.into_val(&env),
        1_000_000u64.into_val(&env),
    ];
    let _ = env.try_invoke_contract::<Val, soroban_sdk::Error>(
        &registry,
        &Symbol::new(&env, "consent_attestation"),
        consent,
    );
    report.measure(
        &env,
        "attestation-registry/attest",
        &registry,
        "attest",
        vec![
            &env,
            attester.into_val(&env),
            patient.into_val(&env),
            record.into_val(&env),
        ],
    );
    report.measure(
        &env,
        "attestation-registry/get_attestation",
        &registry,
        "get_attestation",
        vec![&env, record.into_val(&env)],
    );
    report.measure(
        &env,
        "attestation-registry/is_verified",
        &registry,
        "is_verified",
        vec![&env, record.into_val(&env)],
    );
    Ok(())
}

fn multisig_account(dir: &Path, report: &mut Report) -> anyhow::Result<()> {
    let wasm = load(dir, report, "multisig-account")?;
    let env = new_env();
    let signers: Vec<BytesN<32>> =
        (1..=3u8)
            .map(|i| BytesN::from_array(&env, &[i; 32]))
            .fold(Vec::new(&env), |mut v, k| {
                v.push_back(k);
                v
            });
    let account = deploy(
        &env,
        &wasm,
        vec![&env, signers.into_val(&env), 2u32.into_val(&env)],
    );
    report.measure(
        &env,
        "multisig-account/get_signers",
        &account,
        "get_signers",
        vec![&env],
    );
    report.measure(
        &env,
        "multisig-account/set_signers",
        &account,
        "set_signers",
        vec![&env, signers.into_val(&env), 3u32.into_val(&env)],
    );
    Ok(())
}

fn incentive_pool(dir: &Path, report: &mut Report) -> anyhow::Result<()> {
    let attesters = std::fs::read(dir.join("attester_registry.wasm"))?;
    let wasm = load(dir, report, "incentive-pool")?;
    let env = new_env();
    let admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let attester_registry = deploy(&env, &attesters, vec![&env, admin.into_val(&env)]);
    let pool = env.register(wasm.as_slice(), ());
    StellarAssetClient::new(&env, &token).mint(&admin, &1_000_000);
    let init = vec![
        &env,
        admin.into_val(&env),
        admin.into_val(&env),
        token.into_val(&env),
        attester_registry.into_val(&env),
        1_000i128.into_val(&env),
        10_000i128.into_val(&env),
    ];
    if report.measure(&env, "incentive-pool/initialize", &pool, "initialize", init) {
        report.measure(
            &env,
            "incentive-pool/fund",
            &pool,
            "fund",
            vec![&env, 100_000i128.into_val(&env)],
        );
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // Setup panics are reported as errors; keep stderr quiet about them.
    panic::set_hook(Box::new(|_| {}));
    let mut report = Report::default();
    for (contract, run) in [
        (
            "attester-registry",
            attester_registry as fn(&Path, &mut Report) -> anyhow::Result<()>,
        ),
        ("attestation-registry", attestation_registry),
        ("multisig-account", multisig_account),
        ("incentive-pool", incentive_pool),
    ] {
        // A contract missing from the build (it did not exist yet at the
        // measured commit) or whose constructor differs is an error entry,
        // not a harness failure.
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| run(&cli.wasm_dir, &mut report)));
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                report
                    .errors
                    .insert(contract.to_string(), format!("{err:#}"));
            }
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                let msg = msg.lines().next().unwrap_or_default();
                report
                    .errors
                    .insert(contract.to_string(), format!("setup panicked: {msg}"));
            }
        }
    }
    let out = serde_json::json!({
        "schema": 1,
        "metrics": report.metrics,
        "errors": report.errors,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
