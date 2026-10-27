//! `trust verify`: check a domain's SEP-1 `stellar.toml` (docs/specs/stellar-toml.md)
//! against the local network config and on-chain contract instances.

use anyhow::{anyhow, bail, Context};
use lafiya_config::{ContractKind, NetworkConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use stellar_xdr::{
    ContractDataDurability, ContractExecutable, LedgerEntryData, LedgerKey, LedgerKeyContractData,
    Limits, ReadXdr, ScAddress, ScVal, WriteXdr,
};
use ureq::tls::{Certificate, RootCerts, TlsConfig};

/// The parts of a Lafiya `stellar.toml` that `trust verify` checks.
#[derive(Debug, Deserialize)]
pub struct StellarToml {
    #[serde(rename = "LAFIYA_CONTRACTS", default)]
    pub contracts: Vec<ContractEntry>,
}

/// One `[[LAFIYA_CONTRACTS]]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct ContractEntry {
    pub name: String,
    pub network: String,
    pub network_passphrase: String,
    pub contract_id: String,
    pub wasm_hash: String,
}

pub fn stellar_toml_url(domain: &str) -> String {
    format!(
        "https://{}/.well-known/stellar.toml",
        domain.trim_end_matches('/')
    )
}

/// Fetch `https://<domain>/.well-known/stellar.toml`. `ca_pem`, when given,
/// replaces the default WebPKI roots (for staging hosts and tests).
pub fn fetch(domain: &str, ca_pem: Option<&[u8]>) -> anyhow::Result<String> {
    let url = stellar_toml_url(domain);
    let mut config = ureq::config::Config::builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(15)));
    if let Some(pem) = ca_pem {
        let cert = Certificate::from_pem(pem).context("invalid --ca-cert PEM")?;
        config = config.tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::new_with_certs(&[cert]))
                .build(),
        );
    }
    let agent: ureq::Agent = config.build().into();
    agent
        .get(&url)
        .call()
        .with_context(|| format!("failed to fetch {url}"))?
        .body_mut()
        .read_to_string()
        .with_context(|| format!("failed to read {url}"))
}

pub fn parse(text: &str) -> anyhow::Result<StellarToml> {
    toml::from_str(text).context("invalid stellar.toml")
}

/// Compare published entries for `network` with the local config. Returns the
/// published entries that match a local contract, plus every mismatch found.
pub fn compare_with_config(
    published: &StellarToml,
    network: &str,
    cfg: &NetworkConfig,
) -> (Vec<ContractEntry>, Vec<String>) {
    let mut matched = Vec::new();
    let mut problems = Vec::new();
    for kind in [
        ContractKind::AttesterRegistry,
        ContractKind::AttestationRegistry,
    ] {
        let local = cfg.contract_id(kind);
        let entry = published
            .contracts
            .iter()
            .find(|e| e.name == kind.key() && e.network == network);
        match entry {
            None if local.is_empty() => {}
            None => problems.push(format!(
                "{kind}: local config has {local} but stellar.toml lists none for '{network}'"
            )),
            Some(e) if e.network_passphrase != cfg.network_passphrase => problems.push(format!(
                "{kind}: stellar.toml passphrase {:?} does not match local {:?}",
                e.network_passphrase, cfg.network_passphrase
            )),
            Some(e) if e.contract_id != local => problems.push(format!(
                "{kind}: stellar.toml lists {} but local config has {}",
                e.contract_id,
                if local.is_empty() {
                    "<not deployed>"
                } else {
                    local
                }
            )),
            Some(e) => matched.push(e.clone()),
        }
    }
    (matched, problems)
}

/// Hex wasm hash of a deployed contract's instance, via `getLedgerEntries`.
pub fn onchain_wasm_hash(rpc_url: &str, contract_id: &str) -> anyhow::Result<String> {
    let contract: ScAddress = contract_id
        .parse()
        .map_err(|_| anyhow!("invalid contract id {contract_id}"))?;
    let key = LedgerKey::ContractData(LedgerKeyContractData {
        contract,
        key: ScVal::LedgerKeyContractInstance,
        durability: ContractDataDurability::Persistent,
    })
    .to_xdr_base64(Limits::none())?;
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "getLedgerEntries", "params": { "keys": [key] }
    });
    let text = ureq::post(rpc_url)
        .header("content-type", "application/json")
        .send(body.to_string())
        .with_context(|| format!("getLedgerEntries via {rpc_url} failed"))?
        .body_mut()
        .read_to_string()?;
    let reply: Value = serde_json::from_str(&text)?;
    let xdr = reply["result"]["entries"][0]["xdr"]
        .as_str()
        .ok_or_else(|| anyhow!("no contract instance for {contract_id} on chain"))?;
    match LedgerEntryData::from_xdr_base64(xdr, Limits::none())? {
        LedgerEntryData::ContractData(data) => match data.val {
            ScVal::ContractInstance(instance) => match instance.executable {
                ContractExecutable::Wasm(hash) => {
                    Ok(hash.0.iter().map(|b| format!("{b:02x}")).collect())
                }
                ContractExecutable::StellarAsset => bail!("{contract_id} is not a wasm contract"),
            },
            _ => bail!("unexpected instance value for {contract_id}"),
        },
        _ => bail!("unexpected ledger entry for {contract_id}"),
    }
}

// ---------------------------------------------------------------------------
// `doctor`: preflight diagnosis of environment, config, RPC and deployment drift
// ---------------------------------------------------------------------------

/// Severity of a single `doctor` check result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Informational; never fails the run.
    Info,
    /// Should be fixed, but does not block mutating actions.
    Warning,
    /// Blocks mutating actions; makes `doctor` exit non-zero.
    Error,
}

impl Severity {
    pub fn is_blocking(self) -> bool {
        matches!(self, Severity::Error)
    }
}

/// Outcome of one `doctor` check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    /// Stable identifier, e.g. `rpc.health`.
    pub id: String,
    pub severity: Severity,
    /// Human-readable one-line summary.
    pub message: String,
    /// Actionable hint for the operator, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl CheckResult {
    pub fn ok(id: &str, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            severity: Severity::Info,
            message: message.into(),
            remediation: None,
        }
    }

    pub fn warning(id: &str, message: impl Into<String>, remediation: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            severity: Severity::Warning,
            message: message.into(),
            remediation: Some(remediation.into()),
        }
    }

    pub fn error(id: &str, message: impl Into<String>, remediation: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            severity: Severity::Error,
            message: message.into(),
            remediation: Some(remediation.into()),
        }
    }
}

/// Machine-readable `doctor` report.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<CheckResult>,
    /// True when at least one blocking (`error`) check failed.
    pub blocking_failure: bool,
}

impl DoctorReport {
    pub fn new(checks: Vec<CheckResult>) -> Self {
        let blocking_failure = checks.iter().any(|c| c.severity.is_blocking());
        Self {
            checks,
            blocking_failure,
        }
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("doctor report is serializable")
    }
}

/// A single preflight check. Implementations are pure over their provider so
/// they can be unit-tested with mocks.
pub trait Check {
    fn id(&self) -> &'static str;
    fn run(&self) -> CheckResult;
}

/// Everything `doctor` needs from the outside world. Production uses
/// [`LiveProvider`]; tests use a mock.
pub trait DoctorProvider {
    /// Validate the local config schema; returns problems found.
    fn config_problems(&self) -> Vec<String>;
    /// The configured network passphrase.
    fn network_passphrase(&self) -> String;
    /// The passphrase reported by the RPC endpoint, if reachable.
    fn rpc_passphrase(&self) -> anyhow::Result<String>;
    /// RPC protocol version reported by the endpoint.
    fn rpc_protocol_version(&self) -> anyhow::Result<u32>;
    /// Hex wasm hash of the on-chain instance for `contract_id`.
    fn onchain_wasm_hash(&self, contract_id: &str) -> anyhow::Result<String>;
    /// Wasm hash recorded in the deployment ledger for `contract_id`.
    fn ledger_wasm_hash(&self, contract_id: &str) -> Option<String>;
    /// Native XLM balance of the signer account, in stroops.
    fn signer_balance_stroops(&self) -> anyhow::Result<i64>;
    /// Remaining TTL (in ledgers) of the instance entry for `contract_id`.
    fn instance_ttl_remaining(&self, contract_id: &str) -> anyhow::Result<u32>;
    /// Contract ids to check, paired with their [`ContractKind`].
    fn contracts(&self) -> Vec<(ContractKind, String)>;
}

/// Minimum signer balance (in stroops) below which `doctor` warns.
pub const MIN_SIGNER_BALANCE_STROOPS: i64 = 10_000_000; // 1 XLM
/// Instance TTL (in ledgers) below which `doctor` warns.
pub const MIN_INSTANCE_TTL_LEDGERS: u32 = 1_000;

/// Config schema check.
pub struct ConfigSchemaCheck<'a>(pub &'a dyn DoctorProvider);

impl Check for ConfigSchemaCheck<'_> {
    fn id(&self) -> &'static str {
        "config.schema"
    }

    fn run(&self) -> CheckResult {
        let problems = self.0.config_problems();
        if problems.is_empty() {
            CheckResult::ok(self.id(), "config schema is valid")
        } else {
            CheckResult::error(
                self.id(),
                format!("config schema problems: {}", problems.join("; ")),
                "fix the reported fields in your network config file",
            )
        }
    }
}

/// Network passphrase match check.
pub struct PassphraseCheck<'a>(pub &'a dyn DoctorProvider);

impl Check for PassphraseCheck<'_> {
    fn id(&self) -> &'static str {
        "network.passphrase"
    }

    fn run(&self) -> CheckResult {
        match self.0.rpc_passphrase() {
            Err(e) => CheckResult::error(
                self.id(),
                format!("could not read RPC passphrase: {e:#}"),
                "check the RPC URL and network connectivity",
            ),
            Ok(remote) => {
                let local = self.0.network_passphrase();
                if remote == local {
                    CheckResult::ok(self.id(), "network passphrase matches RPC")
                } else {
                    CheckResult::error(
                        self.id(),
                        format!("local passphrase {local:?} does not match RPC {remote:?}"),
                        "set network_passphrase to the RPC's passphrase (testnet resets change it)",
                    )
                }
            }
        }
    }
}

/// Contract hash vs deployment ledger check.
pub struct ContractHashCheck<'a>(pub &'a dyn DoctorProvider);

impl Check for ContractHashCheck<'_> {
    fn id(&self) -> &'static str {
        "contract.hash"
    }

    fn run(&self) -> CheckResult {
        let mut problems = Vec::new();
        for (kind, id) in self.0.contracts() {
            if id.is_empty() {
                continue;
            }
            let onchain = match self.0.onchain_wasm_hash(&id) {
                Ok(h) => h,
                Err(e) => {
                    problems.push(format!("{kind}: cannot read on-chain hash: {e:#}"));
                    continue;
                }
            };
            match self.0.ledger_wasm_hash(&id) {
                None => problems.push(format!(
                    "{kind}: {id} has no deployment ledger entry (stale after reset?)"
                )),
                Some(recorded) if recorded != onchain => problems.push(format!(
                    "{kind}: on-chain hash {onchain} != ledger {recorded}"
                )),
                Some(_) => {}
            }
        }
        if problems.is_empty() {
            CheckResult::ok(self.id(), "contract hashes match deployment ledger")
        } else {
            CheckResult::error(
                self.id(),
                problems.join("; "),
                "re-run `deploy` and update the deployment ledger / config contract ids",
            )
        }
    }
}

/// Signer balance check.
pub struct SignerBalanceCheck<'a>(pub &'a dyn DoctorProvider);

impl Check for SignerBalanceCheck<'_> {
    fn id(&self) -> &'static str {
        "signer.balance"
    }

    fn run(&self) -> CheckResult {
        match self.0.signer_balance_stroops() {
            Err(e) => CheckResult::error(
                self.id(),
                format!("could not read signer balance: {e:#}"),
                "verify the signer secret and RPC connectivity",
            ),
            Ok(balance) if balance < MIN_SIGNER_BALANCE_STROOPS => CheckResult::warning(
                self.id(),
                format!("signer balance {balance} stroops is below {MIN_SIGNER_BALANCE_STROOPS}"),
                "fund the signer account via friendbot or a transfer",
            ),
            Ok(balance) => CheckResult::ok(self.id(), format!("signer balance {balance} stroops")),
        }
    }
}

/// Instance TTL remaining check.
pub struct InstanceTtlCheck<'a>(pub &'a dyn DoctorProvider);

impl Check for InstanceTtlCheck<'_> {
    fn id(&self) -> &'static str {
        "instance.ttl"
    }

    fn run(&self) -> CheckResult {
        let mut problems = Vec::new();
        for (kind, id) in self.0.contracts() {
            if id.is_empty() {
                continue;
            }
            match self.0.instance_ttl_remaining(&id) {
                Err(e) => problems.push(format!("{kind}: cannot read TTL: {e:#}")),
                Ok(ttl) if ttl < MIN_INSTANCE_TTL_LEDGERS => {
                    problems.push(format!("{kind}: only {ttl} ledgers of TTL remain"))
                }
                Ok(_) => {}
            }
        }
        if problems.is_empty() {
            CheckResult::ok(self.id(), "instance TTLs are healthy")
        } else {
            CheckResult::warning(
                self.id(),
                problems.join("; "),
                "extend the instance TTL with `extend-instance` before it expires",
            )
        }
    }
}

/// Run every preflight check and assemble a [`DoctorReport`].
pub fn run_doctor(provider: &dyn DoctorProvider) -> DoctorReport {
    let checks: Vec<CheckResult> = vec![
        ConfigSchemaCheck(provider).run(),
        PassphraseCheck(provider).run(),
        ContractHashCheck(provider).run(),
        SignerBalanceCheck(provider).run(),
        InstanceTtlCheck(provider).run(),
    ];
    DoctorReport::new(checks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lafiya_config::ContractIds;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    const ATTESTER_ID: &str = "CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA";
    const ATTESTATION_ID: &str = "CCWPKEVBYEEDBMX2T4AKBOTTPXCGWNTZQXBOQWOHLVJ7JOWAMX3G6EAX";
    const PASSPHRASE: &str = "Test SDF Network ; September 2015";

    fn published(attester_id: &str) -> String {
        format!(
            r#"
[DOCUMENTATION]
ORG_NAME = "Lafiya"

[[LAFIYA_CONTRACTS]]
name = "attester_registry"
network = "testnet"
network_passphrase = "{PASSPHRASE}"
contract_id = "{attester_id}"
wasm_hash = "{}"
"#,
            "07".repeat(32)
        )
    }

    fn cfg(attester: &str, attestation: &str) -> NetworkConfig {
        NetworkConfig {
            rpc_url: "https://soroban-testnet.stellar.org".into(),
            rpc_urls: Vec::new(),
            network_passphrase: PASSPHRASE.into(),
            contracts: ContractIds {
                attester_registry: attester.into(),
                attestation_registry: attestation.into(),
            },
        }
    }

    /// Serve `body` once over HTTPS with a fresh self-signed certificate for
    /// `localhost`. Returns the domain (`localhost:<port>`) and the cert PEM.
    fn https_fixture(body: String) -> (String, String) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pem = cert.cert.pem();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(cert.key_pair.serialize_der().into()),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let conn = rustls::ServerConnection::new(Arc::new(config)).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, tcp);
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match tls.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            assert!(request.starts_with(b"GET /.well-known/stellar.toml "));
            let _ = write!(
                tls,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = tls.flush();
        });
        (format!("localhost:{port}"), pem)
    }

    #[test]
    fn fetches_and_verifies_over_local_https() {
        let (domain, pem) = https_fixture(published(ATTESTER_ID));
        let text = fetch(&domain, Some(pem.as_bytes())).unwrap();
        let toml = parse(&text).unwrap();
        let (matched, problems) = compare_with_config(&toml, "testnet", &cfg(ATTESTER_ID, ""));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].wasm_hash, "07".repeat(32));
    }

    #[test]
    fn reports_contract_id_mismatch_from_local_https() {
        let (domain, pem) = https_fixture(published(ATTESTATION_ID));
        let toml = parse(&fetch(&domain, Some(pem.as_bytes())).unwrap()).unwrap();
        let (_, problems) = compare_with_config(&toml, "testnet", &cfg(ATTESTER_ID, ""));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains(ATTESTATION_ID), "{problems:?}");
    }

    #[test]
    fn untrusted_certificate_is_rejected() {
        let (domain, _) = https_fixture(published(ATTESTER_ID));
        assert!(fetch(&domain, None).is_err());
    }

    #[test]
    fn reports_missing_and_mismatched_passphrase_entries() {
        let toml = parse(&published(ATTESTER_ID)).unwrap();
        let (_, problems) =
            compare_with_config(&toml, "testnet", &cfg(ATTESTER_ID, ATTESTATION_ID));
        assert!(
            problems[0].contains("attestation_registry") && problems[0].contains("lists none"),
            "{problems:?}"
        );

        let mut other = cfg(ATTESTER_ID, "");
        other.network_passphrase = "Other".into();
        let (_, problems) = compare_with_config(&toml, "testnet", &other);
        assert!(problems[0].contains("passphrase"), "{problems:?}");
    }

    #[test]
    fn undeployed_network_with_no_entries_is_clean() {
        let toml = parse(&published(ATTESTER_ID)).unwrap();
        let (matched, problems) = compare_with_config(&toml, "futurenet", &cfg("", ""));
        assert!(matched.is_empty() && problems.is_empty());
    }

    #[tokio::test]
    async fn reads_wasm_hash_from_contract_instance() {
        use stellar_xdr::{
            ContractDataEntry, ExtensionPoint, Hash, LedgerEntryData, ScContractInstance,
        };
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let entry = LedgerEntryData::ContractData(ContractDataEntry {
            ext: ExtensionPoint::V0,
            contract: ATTESTER_ID.parse().unwrap(),
            key: ScVal::LedgerKeyContractInstance,
            durability: ContractDataDurability::Persistent,
            val: ScVal::ContractInstance(ScContractInstance {
                executable: ContractExecutable::Wasm(Hash([7; 32])),
                storage: None,
            }),
        })
        .to_xdr_base64(Limits::none())
        .unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 1,
                "result": { "entries": [{ "xdr": entry }], "latestLedger": 1 }
            })))
            .mount(&server)
            .await;

        let url = server.uri();
        let hash = tokio::task::spawn_blocking(move || onchain_wasm_hash(&url, ATTESTER_ID))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hash, "07".repeat(32));
    }

    // -- doctor ------------------------------------------------------------

    /// Mock provider with sensible healthy defaults; tests override fields.
    struct MockProvider {
        config_problems: Vec<String>,
        local_passphrase: String,
        rpc_passphrase: anyhow::Result<String>,
        onchain_hash: anyhow::Result<String>,
        ledger_hash: Option<String>,
        balance: anyhow::Result<i64>,
        ttl: anyhow::Result<u32>,
        contracts: Vec<(ContractKind, String)>,
    }

    impl Default for MockProvider {
        fn default() -> Self {
            Self {
                config_problems: Vec::new(),
                local_passphrase: PASSPHRASE.into(),
                rpc_passphrase: Ok(PASSPHRASE.into()),
                onchain_hash: Ok("07".repeat(32)),
                ledger_hash: Some("07".repeat(32)),
                balance: Ok(MIN_SIGNER_BALANCE_STROOPS * 10),
                ttl: Ok(MIN_INSTANCE_TTL_LEDGERS * 10),
                contracts: vec![(ContractKind::AttesterRegistry, ATTESTER_ID.into())],
            }
        }
    }

    impl DoctorProvider for MockProvider {
        fn config_problems(&self) -> Vec<String> {
            self.config_problems.clone()
        }
        fn network_passphrase(&self) -> String {
            self.local_passphrase.clone()
        }
        fn rpc_passphrase(&self) -> anyhow::Result<String> {
            self.rpc_passphrase.clone()
        }
        fn rpc_protocol_version(&self) -> anyhow::Result<u32> {
            Ok(20)
        }
        fn onchain_wasm_hash(&self, _contract_id: &str) -> anyhow::Result<String> {
            self.onchain_hash.clone()
        }
        fn ledger_wasm_hash(&self, _contract_id: &str) -> Option<String> {
            self.ledger_hash.clone()
        }
        fn signer_balance_stroops(&self) -> anyhow::Result<i64> {
            self.balance.clone()
        }
        fn instance_ttl_remaining(&self, _contract_id: &str) -> anyhow::Result<u32> {
            self.ttl.clone()
        }
        fn contracts(&self) -> Vec<(ContractKind, String)> {
            self.contracts.clone()
        }
    }

    #[test]
    fn doctor_healthy_network_has_no_blocking_failure() {
        let report = run_doctor(&MockProvider::default());
        assert!(!report.blocking_failure, "{:?}", report.checks);
        assert!(report.checks.iter().all(|c| !c.severity.is_blocking()));
    }

    #[test]
    fn config_schema_check_flags_problems() {
        let provider = MockProvider {
            config_problems: vec!["rpc_url is empty".into()],
            ..Default::default()
        };
        let result = ConfigSchemaCheck(&provider).run();
        assert_eq!(result.id, "config.schema");
        assert!(result.severity.is_blocking());
        assert!(result.message.contains("rpc_url is empty"));
        assert!(result.remediation.is_some());
    }

    #[test]
    fn passphrase_check_detects_mismatch() {
        let provider = MockProvider {
            local_passphrase: "Other".into(),
            ..Default::default()
        };
        let result = PassphraseCheck(&provider).run();
        assert_eq!(result.id, "network.passphrase");
        assert!(result.severity.is_blocking());
        assert!(result.message.contains("does not match"));
    }

    #[test]
    fn contract_hash_check_detects_drift() {
        let provider = MockProvider {
            onchain_hash: Ok("08".repeat(32)),
            ..Default::default()
        };
        let result = ContractHashCheck(&provider).run();
        assert_eq!(result.id, "contract.hash");
        assert!(result.severity.is_blocking());
        assert!(result.message.contains("!="));
    }

    #[test]
    fn contract_hash_check_flags_missing_ledger_entry() {
        let provider = MockProvider {
            ledger_hash: None,
            ..Default::default()
        };
        let result = ContractHashCheck(&provider).run();
        assert!(result.severity.is_blocking());
        assert!(result.message.contains("no deployment ledger entry"));
    }

    #[test]
    fn signer_balance_check_warns_when_low() {
        let provider = MockProvider {
            balance: Ok(1),
            ..Default::default()
        };
        let result = SignerBalanceCheck(&provider).run();
        assert_eq!(result.id, "signer.balance");
        assert_eq!(result.severity, Severity::Warning);
        assert!(result.remediation.is_some());
    }

    #[test]
    fn instance_ttl_check_warns_when_low() {
        let provider = MockProvider {
            ttl: Ok(1),
            ..Default::default()
        };
        let result = InstanceTtlCheck(&provider).run();
        assert_eq!(result.id, "instance.ttl");
        assert_eq!(result.severity, Severity::Warning);
        assert!(result.message.contains("TTL"));
    }

    #[test]
    fn doctor_report_is_machine_readable_and_blocks_on_error() {
        let provider = MockProvider {
            config_problems: vec!["bad".into()],
            ..Default::default()
        };
        let report = run_doctor(&provider);
        assert!(report.blocking_failure);
        let json = report.to_json();
        assert_eq!(json["blocking_failure"], true);
        let checks = json["checks"].as_array().unwrap();
        assert!(checks.iter().any(|c| c["severity"] == "error"));
        assert!(checks.iter().any(|c| c["remediation"].is_string()));
    }
}
