//! `trust verify`: check a domain's SEP-1 `stellar.toml` (docs/specs/stellar-toml.md)
//! against the local network config and on-chain contract instances.

use anyhow::{anyhow, bail, Context};
use lafiya_config::{ContractKind, NetworkConfig};
use serde::Deserialize;
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
}
