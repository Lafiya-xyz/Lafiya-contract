//! `cargo xtask sandbox`: a seeded local chain for `lafiya-web`, `lafiya-verifier`,
//! and indexer development.
//!
//! `up` starts (or reuses) a `stellar/quickstart` container, deploys both
//! registries with deterministic DEV-ONLY keys, and converges the on-chain state
//! to a declarative scenario file. Applying a scenario is idempotent: the current
//! state is read back first and only the missing steps are submitted.
//!
//! The keys derive from public, fixed seeds. Never fund them on a real network.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use lafiya_commitment::{commit_v1, FieldValue};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONTAINER: &str = "lafiya-sandbox";
const IMAGE: &str = "stellar/quickstart:latest";
const RPC_URL: &str = "http://localhost:8000/soroban/rpc";
const FRIENDBOT_URL: &str = "http://localhost:8000/friendbot";
const PASSPHRASE: &str = "Standalone Network ; February 2017";
const KEY_PREFIX: &str = "lafiya-sandbox-";
const SEED_DOMAIN: &str = "lafiya-sandbox/DEV-ONLY/";
const WASM_DIR: &str = "target/wasm32v1-none/release";
/// Parallel transaction submitters (one source account each, so no sequence clashes).
const WORKERS: usize = 50;
/// `attester-registry`'s `BATCH_LIMIT` for `add_attesters`.
const BATCH_LIMIT: usize = 40;

#[derive(Subcommand, Debug)]
pub enum SandboxSub {
    /// Start the local chain, deploy, and apply a scenario (idempotent)
    Up {
        /// Scenario name in <dir>/scenarios/ or a path to a .toml file
        #[arg(long, default_value = "minimal")]
        scenario: String,
        /// Sandbox directory holding scenarios/ and the generated files
        #[arg(long, default_value = "sandbox")]
        dir: PathBuf,
    },
    /// Check that the chain matches a scenario; exit non-zero on any drift
    Verify {
        #[arg(long, default_value = "minimal")]
        scenario: String,
        #[arg(long, default_value = "sandbox")]
        dir: PathBuf,
    },
    /// Stop and remove the local chain container
    Down {
        #[arg(long, default_value = "sandbox")]
        dir: PathBuf,
    },
    /// `down`, forget generated files, then `up` from scratch
    Reset {
        #[arg(long, default_value = "minimal")]
        scenario: String,
        #[arg(long, default_value = "sandbox")]
        dir: PathBuf,
    },
}

pub fn run(sub: &SandboxSub) -> Result<()> {
    match sub {
        SandboxSub::Up { scenario, dir } => up(dir, scenario),
        SandboxSub::Verify { scenario, dir } => verify(dir, scenario),
        SandboxSub::Down { dir } => down(dir),
        SandboxSub::Reset { scenario, dir } => {
            down(dir)?;
            up(dir, scenario)
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario files
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    #[serde(default)]
    pub admin: AdminSpec,
    #[serde(default)]
    pub attesters: Vec<AttesterSpec>,
    #[serde(default)]
    pub attestations: Vec<RecordSpec>,
    /// Records that are attested by `by` and then revoked.
    #[serde(default)]
    pub revocations: Vec<RecordSpec>,
    /// Bulk-generated attesters and attestations (e.g. the `pilot` scenario).
    pub generate: Option<Generate>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct AdminSpec {
    /// Key that deploys and initializes both registries.
    #[serde(default = "default_admin")]
    pub initial: String,
    /// If set and different from `initial`, admin is transferred to this key.
    pub current: Option<String>,
}

impl Default for AdminSpec {
    fn default() -> Self {
        Self {
            initial: default_admin(),
            current: None,
        }
    }
}

fn default_admin() -> String {
    "admin".into()
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Active,
    Suspended,
    Removed,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct AttesterSpec {
    pub name: String,
    /// Attesters with a region are added with metadata, one transaction each;
    /// those without are added in `add_attesters` batches.
    pub region: Option<String>,
    #[serde(default)]
    pub status: Status,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RecordSpec {
    pub record: String,
    pub by: Vec<String>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Generate {
    pub attesters: usize,
    pub attestations: usize,
}

impl Scenario {
    pub fn parse(text: &str) -> Result<Self> {
        let mut s: Scenario = toml::from_str(text).context("invalid scenario file")?;
        s.expand();
        s.validate()?;
        Ok(s)
    }

    fn expand(&mut self) {
        let Some(g) = self.generate.take() else {
            return;
        };
        let first = self.attesters.len();
        for i in 0..g.attesters {
            self.attesters.push(AttesterSpec {
                name: format!("chw-gen-{i:03}"),
                region: None,
                status: Status::Active,
            });
        }
        for j in 0..g.attestations {
            self.attestations.push(RecordSpec {
                record: format!("patient-gen-{j:04}"),
                by: vec![self.attesters[first + j % g.attesters].name.clone()],
            });
        }
    }

    fn validate(&self) -> Result<()> {
        let names: BTreeSet<&str> = self.attesters.iter().map(|a| a.name.as_str()).collect();
        if names.len() != self.attesters.len() {
            bail!("duplicate attester name in scenario");
        }
        let mut records = BTreeSet::new();
        for r in self.attestations.iter().chain(&self.revocations) {
            if !records.insert(r.record.as_str()) {
                bail!("record `{}` listed more than once", r.record);
            }
            if r.by.is_empty() {
                bail!("record `{}` has no attesters", r.record);
            }
            for by in &r.by {
                if !names.contains(by.as_str()) {
                    bail!("record `{}`: unknown attester `{by}`", r.record);
                }
            }
        }
        for a in &self.attesters {
            if a.region
                .as_ref()
                .is_some_and(|r| r.is_empty() || r.len() > 32)
            {
                bail!("attester `{}`: region must be 1-32 chars", a.name);
            }
        }
        Ok(())
    }

    pub fn final_admin(&self) -> &str {
        self.admin.current.as_deref().unwrap_or(&self.admin.initial)
    }

    /// Every key the scenario needs, admins first.
    pub fn key_names(&self) -> Vec<String> {
        let mut names = vec![self.admin.initial.clone()];
        if self.final_admin() != self.admin.initial {
            names.push(self.final_admin().to_string());
        }
        names.extend(self.attesters.iter().map(|a| a.name.clone()));
        names
    }
}

// ---------------------------------------------------------------------------
// Deterministic demo data
// ---------------------------------------------------------------------------

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// 32-char seed for `stellar keys generate --seed` (32 bytes of BIP-39 entropy).
pub fn dev_seed(name: &str) -> String {
    hex(&sha256(&[SEED_DOMAIN.as_bytes(), name.as_bytes()]))[..32].to_string()
}

/// A plaintext demo record, its salt, and its LRC-1 commitment.
pub struct DemoRecord {
    pub fields: [(&'static str, String); 3],
    pub salt: [u8; 32],
    pub hash: [u8; 32],
}

pub fn demo_record(record: &str) -> DemoRecord {
    const BLOOD: [&str; 4] = ["O+", "A+", "B+", "AB-"];
    const ALLERGIES: [&str; 3] = ["none", "penicillin", "peanuts"];
    let d = sha256(&[b"lafiya-sandbox/record/", record.as_bytes()]);
    let salt = sha256(&[b"lafiya-sandbox/salt/", record.as_bytes()]);
    let fields = [
        ("patient_ref", record.to_string()),
        (
            "blood_group",
            BLOOD[d[0] as usize % BLOOD.len()].to_string(),
        ),
        (
            "allergies",
            ALLERGIES[d[1] as usize % ALLERGIES.len()].to_string(),
        ),
    ];
    let mut values: Vec<FieldValue> = fields
        .iter()
        .map(|(_, v)| FieldValue::Text(v.clone()))
        .collect();
    values.push(FieldValue::Bytes(salt.to_vec()));
    DemoRecord {
        hash: commit_v1(&values),
        fields,
        salt,
    }
}

// ---------------------------------------------------------------------------
// Planning (pure, so idempotency is unit-testable)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnChain {
    Active,
    Suspended,
}

/// On-chain state, with addresses already mapped back to scenario key names.
#[derive(Debug, Default, Clone)]
pub struct Observed {
    /// `None` until the registries are initialized.
    pub admin: Option<String>,
    pub attesters: BTreeMap<String, OnChain>,
    /// Record name -> attesters in its (bounded) history.
    pub history: BTreeMap<String, Vec<String>>,
    /// Records whose revocation was already applied on this deployment.
    pub revoked: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Initialize,
    Add(String),
    Reinstate(String),
    Attest { by: String, record: String },
    Revoke(String),
    Suspend(String),
    Remove(String),
    TransferAdmin(String),
}

/// `attestation-registry` keeps only the latest `MAX_HISTORY` entries per record.
const MAX_HISTORY: usize = 10;

fn missing_attestations(r: &RecordSpec, have: &[String]) -> Vec<Step> {
    if r.by.len() > MAX_HISTORY && have.len() == MAX_HISTORY {
        return Vec::new();
    }
    let mut have: Vec<&String> = have.iter().collect();
    let mut out = Vec::new();
    for by in &r.by {
        if let Some(i) = have.iter().position(|h| *h == by) {
            have.swap_remove(i);
        } else {
            out.push(Step::Attest {
                by: by.clone(),
                record: r.record.clone(),
            });
        }
    }
    out
}

pub fn plan(s: &Scenario, obs: &Observed) -> Vec<Step> {
    let mut steps = Vec::new();
    if obs.admin.is_none() {
        steps.push(Step::Initialize);
    }
    let empty = Vec::new();
    let history = |r: &str| obs.history.get(r).unwrap_or(&empty);

    let mut attests = Vec::new();
    for r in &s.attestations {
        attests.extend(missing_attestations(r, history(&r.record)));
    }
    let mut revokes = Vec::new();
    for r in &s.revocations {
        let h = history(&r.record);
        if !obs.revoked.contains(&r.record) {
            attests.extend(missing_attestations(r, h));
            revokes.push(Step::Revoke(r.record.clone()));
        } else if !h.is_empty() {
            revokes.push(Step::Revoke(r.record.clone()));
        }
    }
    let workers: BTreeSet<&str> = attests
        .iter()
        .filter_map(|st| match st {
            Step::Attest { by, .. } => Some(by.as_str()),
            _ => None,
        })
        .collect();

    // Phase 1: make every attester that exists or has work to do present and
    // (if needed) active.
    let mut present = BTreeMap::new();
    for a in &s.attesters {
        let busy = workers.contains(a.name.as_str());
        let cur = obs.attesters.get(&a.name).copied();
        let now = match cur {
            None if busy || a.status != Status::Removed => {
                steps.push(Step::Add(a.name.clone()));
                Some(OnChain::Active)
            }
            Some(OnChain::Suspended) if busy || a.status == Status::Active => {
                steps.push(Step::Reinstate(a.name.clone()));
                Some(OnChain::Active)
            }
            other => other,
        };
        present.insert(a.name.as_str(), now);
    }

    // Phase 2: attestations, then revocations.
    steps.extend(attests);
    steps.extend(revokes);

    // Phase 3: final attester lifecycle states.
    for a in &s.attesters {
        match (a.status, present[a.name.as_str()]) {
            (Status::Suspended, Some(OnChain::Active)) => steps.push(Step::Suspend(a.name.clone())),
            (Status::Removed, Some(_)) => steps.push(Step::Remove(a.name.clone())),
            _ => {}
        }
    }

    // Phase 4: admin transfer last, so the admin steps above run as one key.
    let admin_now = obs.admin.as_deref().unwrap_or(&s.admin.initial);
    if admin_now != s.final_admin() {
        steps.push(Step::TransferAdmin(s.final_admin().to_string()));
    }
    steps
}

// ---------------------------------------------------------------------------
// Chain access through the stellar CLI
// ---------------------------------------------------------------------------

fn key_id(name: &str) -> String {
    format!("{KEY_PREFIX}{name}")
}

fn cmd_output(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .output()
        .with_context(|| format!("failed to run {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

struct Chain {
    attester_registry: String,
    attestation_registry: String,
    addresses: BTreeMap<String, String>,
}

fn invoke(
    contract: &str,
    source: &str,
    send: bool,
    func: &str,
    args: &[(&str, &str)],
) -> Result<String> {
    let mut c = Command::new("stellar");
    c.args(["contract", "invoke", "--id", contract, "--source"])
        .arg(key_id(source))
        .args(["--rpc-url", RPC_URL, "--network-passphrase", PASSPHRASE]);
    if !send {
        c.arg("--send=no");
    }
    c.args(["--", func]);
    for (k, v) in args {
        c.arg(format!("--{k}")).arg(v);
    }
    // Reads are safe to retry; the local RPC occasionally fails simulations
    // under heavy parallel load.
    let attempts = if send { 1 } else { 5 };
    let mut result = cmd_output(&mut c);
    for i in 1..attempts {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(i));
        result = cmd_output(&mut c);
    }
    result
}

fn json(out: &str) -> Result<serde_json::Value> {
    serde_json::from_str(out).with_context(|| format!("unexpected CLI output: {out}"))
}

impl Chain {
    fn name_of(&self, address: &str) -> Option<String> {
        self.addresses
            .iter()
            .find(|(_, a)| *a == address)
            .map(|(n, _)| n.clone())
    }

    fn addr(&self, name: &str) -> &str {
        &self.addresses[name]
    }

    fn observe(&self, s: &Scenario, dir: &Path) -> Result<Observed> {
        let reader = &s.admin.initial;
        let mut obs = Observed::default();
        if let Ok(out) = invoke(&self.attester_registry, reader, false, "get_admin", &[]) {
            let addr = json(&out)?.as_str().unwrap_or_default().to_string();
            obs.admin = Some(
                self.name_of(&addr)
                    .with_context(|| format!("admin {addr} is not a sandbox key"))?,
            );
        }
        if obs.admin.is_none() {
            return Ok(obs);
        }
        let statuses = parallel(&s.attesters, |a| {
            let out = invoke(
                &self.attester_registry,
                reader,
                false,
                "get_attester_status",
                &[("attester", self.addr(&a.name))],
            )?;
            let v = json(&out)?;
            Ok(match v.get("suspended").and_then(|b| b.as_bool()) {
                None => None,
                Some(true) => Some(OnChain::Suspended),
                Some(false) => Some(OnChain::Active),
            })
        })?;
        for (a, st) in s.attesters.iter().zip(statuses) {
            if let Some(st) = st {
                obs.attesters.insert(a.name.clone(), st);
            }
        }
        let records: Vec<&RecordSpec> = s.attestations.iter().chain(&s.revocations).collect();
        let histories = parallel(&records, |r| {
            let out = invoke(
                &self.attestation_registry,
                reader,
                false,
                "get_attestation_history",
                &[("record_hash", &hex(&demo_record(&r.record).hash))],
            )?;
            let v = json(&out)?;
            Ok(v.as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|e| e["attester"].as_str())
                        .map(|a| self.name_of(a).unwrap_or_else(|| a.to_string()))
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default())
        })?;
        for (r, h) in records.iter().zip(histories) {
            if !h.is_empty() {
                obs.history.insert(r.record.clone(), h);
            }
        }
        obs.revoked = read_revoked(dir, &self.attestation_registry);
        Ok(obs)
    }

    fn apply(&self, s: &Scenario, steps: &[Step], dir: &Path) -> Result<()> {
        let mut admin = s.admin.initial.clone();
        let mut attests = Vec::new();
        let mut batch = Vec::new();
        let reg = &self.attester_registry;
        for step in steps {
            if let Step::Add(name) = step {
                let a = s.attesters.iter().find(|a| &a.name == name).expect("known");
                if a.region.is_none() {
                    batch.push(self.addr(name).to_string());
                    continue;
                }
            }
            self.add_batch(&admin, std::mem::take(&mut batch))?;
            if let Step::Attest { by, record } = step {
                attests.push((by.clone(), record.clone()));
                continue;
            }
            // Admin steps are ordered after every attestation they depend on.
            if !attests.is_empty() {
                self.attest_all(std::mem::take(&mut attests))?;
            }
            match step {
                Step::Initialize => {
                    invoke(
                        reg,
                        &admin,
                        true,
                        "initialize",
                        &[("admin", self.addr(&admin))],
                    )?;
                    invoke(
                        &self.attestation_registry,
                        &admin,
                        true,
                        "initialize",
                        &[("admin", self.addr(&admin)), ("attester_registry", reg)],
                    )?;
                }
                Step::Add(name) => {
                    let a = s.attesters.iter().find(|a| &a.name == name).expect("known");
                    let region = a.region.as_deref().unwrap_or_default();
                    let license = hex(&sha256(&[b"lafiya-sandbox/license/", name.as_bytes()]));
                    invoke(
                        reg,
                        &admin,
                        true,
                        "add_attester_with_info",
                        &[
                            ("attester", self.addr(name)),
                            ("license_hash", &format!("\"{license}\"")),
                            ("region", &format!("\"{region}\"")),
                        ],
                    )?;
                }
                Step::Reinstate(n) => {
                    invoke(
                        reg,
                        &admin,
                        true,
                        "reinstate_attester",
                        &[("attester", self.addr(n))],
                    )?;
                }
                Step::Suspend(n) => {
                    invoke(
                        reg,
                        &admin,
                        true,
                        "suspend_attester",
                        &[("attester", self.addr(n))],
                    )?;
                }
                Step::Remove(n) => {
                    invoke(
                        reg,
                        &admin,
                        true,
                        "remove_attester",
                        &[("attester", self.addr(n))],
                    )?;
                }
                Step::Revoke(record) => {
                    invoke(
                        &self.attestation_registry,
                        &admin,
                        true,
                        "revoke_attestation",
                        &[("record_hash", &hex(&demo_record(record).hash))],
                    )?;
                    mark_revoked(dir, &self.attestation_registry, record)?;
                }
                Step::TransferAdmin(to) => {
                    for contract in [reg, &self.attestation_registry] {
                        invoke(
                            contract,
                            &admin,
                            true,
                            "propose_admin",
                            &[("new_admin", self.addr(to))],
                        )?;
                        invoke(contract, to, true, "accept_admin", &[])?;
                    }
                    admin = to.clone();
                }
                Step::Attest { .. } => unreachable!(),
            }
        }
        self.add_batch(&admin, batch)?;
        self.attest_all(attests)
    }

    /// Add metadata-less attesters, `BATCH_LIMIT` per transaction.
    fn add_batch(&self, admin: &str, addrs: Vec<String>) -> Result<()> {
        for chunk in addrs.chunks(BATCH_LIMIT) {
            let list = serde_json::to_string(chunk)?;
            invoke(
                &self.attester_registry,
                admin,
                true,
                "add_attesters",
                &[("attesters", &list)],
            )?;
        }
        Ok(())
    }

    /// Submit attestations in parallel rounds. A round never uses the same
    /// attester twice (source-account sequence numbers) or the same record
    /// twice (its storage footprint changes with every attestation), and
    /// order is preserved per attester and per record.
    fn attest_all(&self, attests: Vec<(String, String)>) -> Result<()> {
        for round in attest_rounds(attests) {
            parallel(&round, |(by, record)| {
                invoke(
                    &self.attestation_registry,
                    by,
                    true,
                    "attest",
                    &[
                        ("attester", self.addr(by)),
                        ("record_hash", &hex(&demo_record(record).hash)),
                    ],
                )
            })?;
        }
        Ok(())
    }
}

fn attest_rounds(mut pending: Vec<(String, String)>) -> Vec<Vec<(String, String)>> {
    let mut rounds = Vec::new();
    while !pending.is_empty() {
        let (mut attesters, mut records) = (BTreeSet::new(), BTreeSet::new());
        let (mut round, mut rest) = (Vec::new(), Vec::new());
        for (by, record) in pending {
            // Both inserts must run, so a blocked item also blocks later ones.
            let free_attester = attesters.insert(by.clone());
            let free_record = records.insert(record.clone());
            if free_attester && free_record {
                round.push((by, record));
            } else {
                rest.push((by, record));
            }
        }
        rounds.push(round);
        pending = rest;
    }
    rounds
}

/// Run `f` over `items` on up to `WORKERS` threads, preserving order.
fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> Result<R> + Sync) -> Result<Vec<R>> {
    let chunk = items.len().div_ceil(WORKERS).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|c| scope.spawn(|| c.iter().map(&f).collect::<Result<Vec<R>>>()))
            .collect();
        let mut out = Vec::with_capacity(items.len());
        for h in handles {
            out.extend(h.join().expect("sandbox worker panicked")?);
        }
        Ok(out)
    })
}

fn revoked_path(dir: &Path, contract: &str) -> PathBuf {
    dir.join(format!(".revoked-{contract}"))
}

fn read_revoked(dir: &Path, contract: &str) -> BTreeSet<String> {
    std::fs::read_to_string(revoked_path(dir, contract))
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn mark_revoked(dir: &Path, contract: &str, record: &str) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(revoked_path(dir, contract))?;
    writeln!(f, "{record}")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn load_scenario(dir: &Path, scenario: &str) -> Result<Scenario> {
    let path = if scenario.ends_with(".toml") {
        PathBuf::from(scenario)
    } else {
        dir.join("scenarios").join(format!("{scenario}.toml"))
    };
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read scenario {}", path.display()))?;
    Scenario::parse(&text).with_context(|| format!("in {}", path.display()))
}

fn start_chain() -> Result<()> {
    let running = cmd_output(Command::new("docker").args([
        "ps",
        "-q",
        "--filter",
        &format!("name=^{CONTAINER}$"),
    ]))?;
    if running.is_empty() {
        let _ = Command::new("docker")
            .args(["rm", "-f", CONTAINER])
            .output();
        println!("starting {IMAGE} as `{CONTAINER}`...");
        cmd_output(Command::new("docker").args([
            "run",
            "-d",
            "--name",
            CONTAINER,
            "-p",
            "8000:8000",
            IMAGE,
            "--local",
            "--limits",
            "unlimited",
        ]))?;
    }
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"getHealth"}"#;
    for _ in 0..90 {
        let health = Command::new("curl")
            .args([
                "-fsS",
                "-H",
                "Content-Type: application/json",
                "-d",
                body,
                RPC_URL,
            ])
            .output();
        if matches!(health, Ok(ref o) if String::from_utf8_lossy(&o.stdout).contains("healthy")) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    bail!("local RPC at {RPC_URL} did not become healthy within 3 minutes")
}

/// Fund `addr` from friendbot, retrying while friendbot is still starting.
fn fund(addr: &str) -> Result<()> {
    for _ in 0..60 {
        let out = Command::new("curl")
            .args(["-sS", &format!("{FRIENDBOT_URL}?addr={addr}")])
            .output()?;
        let body = String::from_utf8_lossy(&out.stdout);
        if body.contains("\"successful\": true") || body.contains("already funded") {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    bail!("friendbot at {FRIENDBOT_URL} could not fund {addr}")
}

fn ensure_keys(names: &[String]) -> Result<BTreeMap<String, String>> {
    let addrs = parallel(names, |name| {
        let id = key_id(name);
        let exists = Command::new("stellar")
            .args(["keys", "address", &id])
            .output()
            .is_ok_and(|o| o.status.success());
        if !exists {
            cmd_output(Command::new("stellar").args([
                "keys",
                "generate",
                &id,
                "--seed",
                &dev_seed(name),
            ]))?;
        }
        let addr = cmd_output(Command::new("stellar").args(["keys", "address", &id]))?;
        fund(&addr)?;
        Ok(addr)
    })?;
    Ok(names.iter().cloned().zip(addrs).collect())
}

fn read_env(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(dir.join(".env.local"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn deploy(admin: &str, crate_name: &str) -> Result<String> {
    let wasm = format!("{WASM_DIR}/{}.wasm", crate_name.replace('-', "_"));
    if !Path::new(&wasm).exists() {
        let status = Command::new("cargo")
            .args([
                "build",
                "--release",
                "--target",
                "wasm32v1-none",
                "-p",
                crate_name,
            ])
            .status()?;
        if !status.success() {
            bail!("building {crate_name} failed");
        }
    }
    cmd_output(Command::new("stellar").args([
        "contract",
        "deploy",
        "--wasm",
        &wasm,
        "--source",
        &key_id(admin),
        "--rpc-url",
        RPC_URL,
        "--network-passphrase",
        PASSPHRASE,
    ]))
}

/// Reuse the contracts recorded in `.env.local` if they still exist on chain.
fn connect(dir: &Path, s: &Scenario, deploy_missing: bool) -> Result<Chain> {
    let addresses = ensure_keys(&s.key_names())?;
    let env = read_env(dir);
    let alive = |id: Option<&String>| {
        id.is_some_and(|id| {
            (0..3).any(|_| {
                Command::new("stellar")
                    .args([
                        "contract",
                        "info",
                        "interface",
                        "--id",
                        id,
                        "--rpc-url",
                        RPC_URL,
                    ])
                    .args(["--network-passphrase", PASSPHRASE])
                    .output()
                    .is_ok_and(|o| o.status.success())
            })
        })
    };
    let (a, b) = (
        env.get("LAFIYA_ATTESTER_REGISTRY_ID"),
        env.get("LAFIYA_ATTESTATION_REGISTRY_ID"),
    );
    let (attester_registry, attestation_registry) = if alive(a) && alive(b) {
        (
            a.cloned().unwrap_or_default(),
            b.cloned().unwrap_or_default(),
        )
    } else if deploy_missing {
        println!("deploying registries...");
        (
            deploy(&s.admin.initial, "attester-registry")?,
            deploy(&s.admin.initial, "attestation-registry")?,
        )
    } else {
        bail!("no sandbox deployment found; run `cargo xtask sandbox up` first");
    };
    Ok(Chain {
        attester_registry,
        attestation_registry,
        addresses,
    })
}

fn write_outputs(dir: &Path, scenario: &str, s: &Scenario, chain: &Chain) -> Result<()> {
    let admin = chain.addr(s.final_admin());
    let mut env = String::from(
        "# DEV ONLY - generated by `cargo xtask sandbox up`. Keys derive from public seeds;\n\
         # never fund or reuse them on a real network.\n",
    );
    let _ = writeln!(env, "LAFIYA_SCENARIO={scenario}");
    let _ = writeln!(env, "LAFIYA_RPC_URL={RPC_URL}");
    let _ = writeln!(env, "LAFIYA_NETWORK_PASSPHRASE={PASSPHRASE}");
    let _ = writeln!(
        env,
        "LAFIYA_ATTESTER_REGISTRY_ID={}",
        chain.attester_registry
    );
    let _ = writeln!(
        env,
        "LAFIYA_ATTESTATION_REGISTRY_ID={}",
        chain.attestation_registry
    );
    let _ = writeln!(env, "LAFIYA_ADMIN_ADDRESS={admin}");
    for (name, addr) in &chain.addresses {
        let var = name
            .to_uppercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        let _ = writeln!(env, "LAFIYA_DEV_KEY_{var}={addr}");
    }
    std::fs::write(dir.join(".env.local"), env)?;

    let records: serde_json::Map<String, serde_json::Value> = s
        .attestations
        .iter()
        .chain(&s.revocations)
        .map(|r| {
            let d = demo_record(&r.record);
            let fields: serde_json::Map<String, serde_json::Value> = d
                .fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone().into()))
                .collect();
            let revoked = s.revocations.iter().any(|x| x.record == r.record);
            (
                r.record.clone(),
                serde_json::json!({
                    "fields": fields,
                    "salt": hex(&d.salt),
                    "record_hash": hex(&d.hash),
                    "attested_by": r.by,
                    "revoked": revoked,
                }),
            )
        })
        .collect();
    let fixture = serde_json::json!({
        "dev_only": true,
        "scenario": scenario,
        "rpc_url": RPC_URL,
        "network_passphrase": PASSPHRASE,
        "contracts": {
            "attester_registry": chain.attester_registry,
            "attestation_registry": chain.attestation_registry,
        },
        "admin": s.final_admin(),
        "accounts": chain.addresses,
        "record_commitment": "LRC-1 over [patient_ref, blood_group, allergies] as Text, then salt as Bytes",
        "records": records,
    });
    std::fs::write(
        dir.join("fixtures.json"),
        serde_json::to_string_pretty(&fixture)? + "\n",
    )?;
    Ok(())
}

fn up(dir: &Path, scenario: &str) -> Result<()> {
    let s = load_scenario(dir, scenario)?;
    let started = std::time::Instant::now();
    start_chain()?;
    let chain = connect(dir, &s, true)?;
    // Record the deployment first so an interrupted run resumes against it.
    write_outputs(dir, scenario, &s, &chain)?;
    let steps = plan(&s, &chain.observe(&s, dir)?);
    println!("applying scenario `{scenario}`: {} change(s)", steps.len());
    chain.apply(&s, &steps, dir)?;
    write_outputs(dir, scenario, &s, &chain)?;
    println!(
        "sandbox ready in {:.0?}; wrote {} and {}",
        started.elapsed(),
        dir.join(".env.local").display(),
        dir.join("fixtures.json").display()
    );
    Ok(())
}

fn verify(dir: &Path, scenario: &str) -> Result<()> {
    let s = load_scenario(dir, scenario)?;
    let chain = connect(dir, &s, false)?;
    let steps = plan(&s, &chain.observe(&s, dir)?);
    if !steps.is_empty() {
        for st in &steps {
            eprintln!("drift: {st:?}");
        }
        bail!("on-chain state does not match scenario `{scenario}`");
    }
    println!("on-chain state matches scenario `{scenario}`");
    Ok(())
}

fn down(dir: &Path) -> Result<()> {
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER])
        .output();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".env.local" || name == "fixtures.json" || name.starts_with(".revoked-") {
            std::fs::remove_file(entry.path())?;
        }
    }
    println!("sandbox stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario(name: &str) -> Scenario {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sandbox/scenarios")
            .join(format!("{name}.toml"));
        Scenario::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// Apply a plan to an in-memory model of the contracts.
    fn simulate(s: &Scenario, obs: &mut Observed, steps: &[Step]) {
        for st in steps {
            match st {
                Step::Initialize => obs.admin = Some(s.admin.initial.clone()),
                Step::Add(n) | Step::Reinstate(n) => {
                    obs.attesters.insert(n.clone(), OnChain::Active);
                }
                Step::Suspend(n) => {
                    obs.attesters.insert(n.clone(), OnChain::Suspended);
                }
                Step::Remove(n) => {
                    obs.attesters.remove(n);
                }
                Step::Attest { by, record } => {
                    assert_eq!(
                        obs.attesters.get(by),
                        Some(&OnChain::Active),
                        "{by} not active"
                    );
                    let h = obs.history.entry(record.clone()).or_default();
                    h.push(by.clone());
                    if h.len() > MAX_HISTORY {
                        h.remove(0);
                    }
                }
                Step::Revoke(r) => {
                    obs.history.remove(r);
                    obs.revoked.insert(r.clone());
                }
                Step::TransferAdmin(to) => obs.admin = Some(to.clone()),
            }
        }
    }

    #[test]
    fn shipped_scenarios_converge_and_are_idempotent() {
        for name in ["minimal", "pilot", "edge-cases"] {
            let s = scenario(name);
            let mut obs = Observed::default();
            let steps = plan(&s, &obs);
            assert!(!steps.is_empty(), "{name}");
            simulate(&s, &mut obs, &steps);
            assert_eq!(plan(&s, &obs), vec![], "{name} is not idempotent");
        }
    }

    #[test]
    fn pilot_is_pilot_sized() {
        let s = scenario("pilot");
        assert!(s.attesters.len() >= 50);
        assert!(s.attestations.len() >= 500);
    }

    #[test]
    fn edge_cases_cover_every_lifecycle_state() {
        let s = scenario("edge-cases");
        for st in [Status::Active, Status::Suspended, Status::Removed] {
            assert!(s.attesters.iter().any(|a| a.status == st), "{st:?}");
        }
        assert!(s.attestations.iter().any(|r| r.by.len() > 1));
        assert!(!s.revocations.is_empty());
        assert_ne!(s.final_admin(), s.admin.initial);
    }

    #[test]
    fn drift_is_repaired() {
        let s = scenario("edge-cases");
        let mut obs = Observed::default();
        let steps = plan(&s, &obs);
        simulate(&s, &mut obs, &steps);
        let suspended = s
            .attesters
            .iter()
            .find(|a| a.status == Status::Suspended)
            .unwrap();
        obs.attesters
            .insert(suspended.name.clone(), OnChain::Active);
        assert_eq!(plan(&s, &obs), vec![Step::Suspend(suspended.name.clone())]);
    }

    #[test]
    fn attest_rounds_never_share_attester_or_record() {
        let s = scenario("edge-cases");
        let steps = plan(&s, &Observed::default());
        let attests: Vec<(String, String)> = steps
            .into_iter()
            .filter_map(|st| match st {
                Step::Attest { by, record } => Some((by, record)),
                _ => None,
            })
            .collect();
        let total = attests.len();
        let rounds = attest_rounds(attests);
        assert_eq!(rounds.iter().map(Vec::len).sum::<usize>(), total);
        for round in rounds {
            let bys: BTreeSet<_> = round.iter().map(|(b, _)| b).collect();
            let records: BTreeSet<_> = round.iter().map(|(_, r)| r).collect();
            assert_eq!(bys.len(), round.len());
            assert_eq!(records.len(), round.len());
        }
    }

    #[test]
    fn rejects_unknown_attester() {
        let err = Scenario::parse(
            "[[attesters]]\nname = \"a\"\nregion = \"NG_LA\"\n\
             [[attestations]]\nrecord = \"r\"\nby = [\"b\"]\n",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("unknown attester `b`"));
    }

    #[test]
    fn dev_data_is_deterministic() {
        assert_eq!(dev_seed("admin"), dev_seed("admin"));
        assert_eq!(dev_seed("admin").len(), 32);
        assert_ne!(dev_seed("admin"), dev_seed("admin-2"));
        assert_eq!(
            demo_record("patient-001").hash,
            demo_record("patient-001").hash
        );
    }
}
