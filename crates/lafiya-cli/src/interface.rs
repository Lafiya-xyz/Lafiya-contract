//! Runtime interface negotiation against a deployed contract.
//!
//! Every Lafiya contract exposes `get_interface() -> InterfaceInfo`. The CLI
//! calls it once per session and refuses to operate, with a clear message,
//! when the contract is of the wrong kind or older than this CLI requires.
//! `contract_kind` is a weak identity check, not proof of authenticity.

use serde::Deserialize;

/// Arguments for the `restore` subcommand: re-fund (restore) an archived
/// attestation entry so proofs over it become readable again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreArgs {
    /// Hex-encoded record hash of the archived attestation.
    pub record_hash: String,
    /// Optional ledger horizon to extend the entry's TTL to. When `None` the
    /// contract applies its default restoration horizon.
    pub extend_to: Option<u32>,
}

impl RestoreArgs {
    /// Parse `restore <record_hash> [--extend-to <ledger>]`.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut record_hash: Option<String> = None;
        let mut extend_to: Option<u32> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--extend-to" {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "--extend-to requires a ledger value".to_string())?;
                let parsed = value
                    .parse::<u32>()
                    .map_err(|e| format!("invalid --extend-to value {value:?}: {e}"))?;
                extend_to = Some(parsed);
                i += 2;
                continue;
            }
            if arg.starts_with("--") {
                return Err(format!("unknown flag {arg}"));
            }
            if record_hash.is_some() {
                return Err(format!("unexpected extra argument {arg}"));
            }
            record_hash = Some(arg.clone());
            i += 1;
        }
        let record_hash = record_hash.ok_or_else(|| "restore requires <record_hash>".to_string())?;
        Ok(Self {
            record_hash,
            extend_to,
        })
    }
}

/// Arguments for the `dispute` subcommand: a patient-signed flag over an
/// attestation commitment. The signature uses the QR-spec patient scheme with
/// domain separation; the CLI only transports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisputeArgs {
    /// Hex-encoded record hash being disputed.
    pub record_hash: String,
    /// Hex-encoded patient signer public key.
    pub signer: String,
    /// Hex-encoded signature over the domain-separated dispute commitment.
    pub signature: String,
}

impl DisputeArgs {
    /// Parse `dispute <record_hash> --signer <key> --signature <sig>`.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut record_hash: Option<String> = None;
        let mut signer: Option<String> = None;
        let mut signature: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            let mut take_value = |name: &str| -> Result<String, String> {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("{name} requires a value"))?;
                Ok(value.clone())
            };
            match arg.as_str() {
                "--signer" => {
                    signer = Some(take_value("--signer")?);
                    i += 2;
                }
                "--signature" => {
                    signature = Some(take_value("--signature")?);
                    i += 2;
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown flag {other}"));
                }
                other => {
                    if record_hash.is_some() {
                        return Err(format!("unexpected extra argument {other}"));
                    }
                    record_hash = Some(other.to_string());
                    i += 1;
                }
            }
        }
        let record_hash =
            record_hash.ok_or_else(|| "dispute requires <record_hash>".to_string())?;
        let signer = signer.ok_or_else(|| "dispute requires --signer <key>".to_string())?;
        let signature =
            signature.ok_or_else(|| "dispute requires --signature <sig>".to_string())?;
        Ok(Self {
            record_hash,
            signer,
            signature,
        })
    }
}

/// Decoded `InterfaceInfo` as printed (JSON) by `stellar contract invoke`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InterfaceInfo {
    pub contract_kind: String,
    pub interface_version: u32,
    pub features: Vec<String>,
    pub schema_version: u32,
    pub event_version: u32,
}

impl InterfaceInfo {
    #[allow(dead_code)]
    pub fn supports(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

/// What this CLI build needs from a contract of a given kind.
#[derive(Debug, Clone, Copy)]
pub struct Requirement {
    pub contract_kind: &'static str,
    pub min_interface_version: u32,
}

pub const ATTESTER_REGISTRY: Requirement = Requirement {
    contract_kind: "lafiya_attester_registry",
    min_interface_version: 1,
};

pub const ATTESTATION_REGISTRY: Requirement = Requirement {
    contract_kind: "lafiya_attestation_registry",
    min_interface_version: 1,
};

pub const INCENTIVE_POOL: Requirement = Requirement {
    contract_kind: "lafiya_incentive_pool",
    min_interface_version: 1,
};

/// Check the raw `get_interface` output against `req`.
///
/// `raw` is `None` when the call failed, which means the contract predates
/// `get_interface` (interface v0).
pub fn negotiate(
    contract_id: &str,
    raw: Option<&str>,
    req: Requirement,
) -> Result<InterfaceInfo, String> {
    let Some(raw) = raw else {
        return Err(format!(
            "contract {contract_id} does not implement get_interface (interface v0); \
             this CLI requires {} >= v{}",
            req.contract_kind, req.min_interface_version
        ));
    };
    let info: InterfaceInfo = serde_json::from_str(raw.trim()).map_err(|e| {
        format!("contract {contract_id} returned an unreadable get_interface result: {e}")
    })?;
    if info.contract_kind != req.contract_kind {
        return Err(format!(
            "contract {contract_id} is a {}, expected {}",
            info.contract_kind, req.contract_kind
        ));
    }
    if info.interface_version < req.min_interface_version {
        return Err(format!(
            "contract {contract_id} is interface v{}; this CLI requires >= v{}",
            info.interface_version, req.min_interface_version
        ));
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = r#"{"contract_kind":"lafiya_attester_registry","interface_version":1,
        "features":["pause","roles"],"schema_version":2,"event_version":1}"#;

    #[test]
    fn accepts_current_contract_and_reports_features() {
        let info = negotiate("CDX", Some(NEW), ATTESTER_REGISTRY).unwrap();
        assert_eq!(info.interface_version, 1);
        assert!(info.supports("pause"));
        assert!(!info.supports("attest_many"));
    }

    #[test]
    fn rejects_old_contract_without_get_interface() {
        let err = negotiate("CDX", None, ATTESTER_REGISTRY).unwrap_err();
        assert!(err.contains("interface v0"), "{err}");
    }

    #[test]
    fn rejects_older_interface_version() {
        let req = Requirement {
            min_interface_version: 3,
            ..ATTESTER_REGISTRY
        };
        let err = negotiate("CDX", Some(NEW), req).unwrap_err();
        assert_eq!(err, "contract CDX is interface v1; this CLI requires >= v3");
    }

    #[test]
    fn rejects_wrong_contract_kind() {
        let err = negotiate("CDX", Some(NEW), ATTESTATION_REGISTRY).unwrap_err();
        assert!(
            err.contains("expected lafiya_attestation_registry"),
            "{err}"
        );
    }

    #[test]
    fn rejects_garbage_output() {
        let err = negotiate("CDX", Some("not json"), ATTESTER_REGISTRY).unwrap_err();
        assert!(err.contains("unreadable"), "{err}");
    }

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_restore_with_and_without_horizon() {
        let bare = RestoreArgs::parse(&argv(&["abcd"])).unwrap();
        assert_eq!(bare.record_hash, "abcd");
        assert_eq!(bare.extend_to, None);

        let with = RestoreArgs::parse(&argv(&["abcd", "--extend-to", "1000"])).unwrap();
        assert_eq!(with.record_hash, "abcd");
        assert_eq!(with.extend_to, Some(1000));
    }

    #[test]
    fn restore_rejects_missing_hash_and_bad_horizon() {
        assert!(RestoreArgs::parse(&argv(&[])).unwrap_err().contains("record_hash"));
        assert!(RestoreArgs::parse(&argv(&["abcd", "--extend-to", "x"]))
            .unwrap_err()
            .contains("invalid --extend-to"));
        assert!(RestoreArgs::parse(&argv(&["abcd", "--bogus"]))
            .unwrap_err()
            .contains("unknown flag"));
    }

    #[test]
    fn parses_dispute_with_signer_and_signature() {
        let args = DisputeArgs::parse(&argv(&[
            "abcd",
            "--signer",
            "key",
            "--signature",
            "sig",
        ]))
        .unwrap();
        assert_eq!(args.record_hash, "abcd");
        assert_eq!(args.signer, "key");
        assert_eq!(args.signature, "sig");
    }

    #[test]
    fn dispute_requires_signer_and_signature() {
        assert!(DisputeArgs::parse(&argv(&["abcd"]))
            .unwrap_err()
            .contains("--signer"));
        assert!(DisputeArgs::parse(&argv(&["abcd", "--signer", "key"]))
            .unwrap_err()
            .contains("--signature"));
    }
}
