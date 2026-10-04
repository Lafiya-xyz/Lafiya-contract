//! Runtime interface negotiation against a deployed contract.
//!
//! Every Lafiya contract exposes `get_interface() -> InterfaceInfo`. The CLI
//! calls it once per session and refuses to operate, with a clear message,
//! when the contract is of the wrong kind or older than this CLI requires.
//! `contract_kind` is a weak identity check, not proof of authenticity.

use serde::Deserialize;

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
}
