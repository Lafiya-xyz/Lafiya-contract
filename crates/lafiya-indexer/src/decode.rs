//! Raw `getEvents` entries and their decoding into typed Lafiya events
//! (schemas in `docs/events.md`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use stellar_xdr::{Limits, ReadXdr, ScVal};

/// A contract event as returned by Soroban RPC `getEvents` (and as read from
/// a backfill export).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawEvent {
    pub id: String,
    pub ledger: u32,
    pub ledger_closed_at: String,
    pub contract_id: String,
    pub tx_hash: String,
    /// Base64 XDR `ScVal` topics.
    pub topic: Vec<String>,
    /// Base64 XDR `ScVal` data.
    pub value: String,
    #[serde(default = "default_true")]
    pub in_successful_contract_call: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LafiyaEvent {
    AttesterAdded {
        attester: String,
    },
    AttesterRemoved {
        attester: String,
    },
    AttesterSuspended {
        attester: String,
    },
    AttesterReinstated {
        attester: String,
    },
    AttesterInfoUpdated {
        attester: String,
    },
    AttestationRecorded {
        record_hash: String,
        attester: String,
        timestamp: u64,
    },
    AttestationRevoked {
        record_hash: String,
    },
    /// Administration events, with their topic fields rendered as strings.
    Admin {
        name: String,
        fields: Map<String, Value>,
    },
    /// An event name this indexer does not know; kept in `raw_events` only.
    Unknown {
        name: String,
    },
}

impl LafiyaEvent {
    pub fn name(&self) -> &str {
        match self {
            Self::AttesterAdded { .. } => "attester_added",
            Self::AttesterRemoved { .. } => "attester_removed",
            Self::AttesterSuspended { .. } => "attester_suspended",
            Self::AttesterReinstated { .. } => "attester_reinstated",
            Self::AttesterInfoUpdated { .. } => "attester_info_updated",
            Self::AttestationRecorded { .. } => "attestation_recorded",
            Self::AttestationRevoked { .. } => "attestation_revoked",
            Self::Admin { name, .. } | Self::Unknown { name } => name,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("event {id}: {reason}")]
pub struct DecodeError {
    pub id: String,
    pub reason: String,
}

/// Topic field names (after the prefix symbol) of administration events.
const ADMIN_EVENTS: &[(&str, &[&str])] = &[
    ("admin_transferred", &["previous_admin", "new_admin"]),
    ("initialized", &["admin"]),
    ("paused", &["by"]),
    ("unpaused", &["by"]),
    ("upgraded", &["new_wasm_hash"]),
    ("attester_registry_repointed", &["previous", "new"]),
];

pub fn decode(raw: &RawEvent) -> Result<LafiyaEvent, DecodeError> {
    let err = |reason: String| DecodeError {
        id: raw.id.clone(),
        reason,
    };
    let topics = raw
        .topic
        .iter()
        .map(|t| parse_scval(t))
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    let name = topics
        .first()
        .and_then(symbol)
        .ok_or_else(|| err("first topic is not a symbol".into()))?;
    let topic = |i: usize| {
        topics
            .get(i)
            .ok_or_else(|| err(format!("missing topic {i}")))
    };
    let address_topic =
        |i| topic(i).and_then(|v| address(v).ok_or_else(|| err("expected address".into())));
    let hash_topic =
        |i| topic(i).and_then(|v| bytes_hex(v).ok_or_else(|| err("expected bytes".into())));

    Ok(match name.as_str() {
        "attester_added" => LafiyaEvent::AttesterAdded {
            attester: address_topic(1)?,
        },
        "attester_removed" => LafiyaEvent::AttesterRemoved {
            attester: address_topic(1)?,
        },
        "attester_suspended" => LafiyaEvent::AttesterSuspended {
            attester: address_topic(1)?,
        },
        "attester_reinstated" => LafiyaEvent::AttesterReinstated {
            attester: address_topic(1)?,
        },
        "attester_info_updated" => LafiyaEvent::AttesterInfoUpdated {
            attester: address_topic(1)?,
        },
        "attestation_revoked" => LafiyaEvent::AttestationRevoked {
            record_hash: hash_topic(1)?,
        },
        "attestation_recorded" => {
            let data = parse_scval(&raw.value).map_err(err)?;
            LafiyaEvent::AttestationRecorded {
                record_hash: hash_topic(1)?,
                attester: map_field(&data, "attester")
                    .and_then(address)
                    .ok_or_else(|| err("data.attester missing".into()))?,
                timestamp: match map_field(&data, "timestamp") {
                    Some(ScVal::U64(t)) => *t,
                    _ => return Err(err("data.timestamp missing".into())),
                },
            }
        }
        other => match ADMIN_EVENTS.iter().find(|(n, _)| *n == other) {
            Some((_, field_names)) => {
                let mut fields = Map::new();
                for (i, field) in field_names.iter().enumerate() {
                    fields.insert((*field).into(), Value::String(render(topic(i + 1)?)));
                }
                LafiyaEvent::Admin {
                    name: other.into(),
                    fields,
                }
            }
            None => LafiyaEvent::Unknown { name: other.into() },
        },
    })
}

pub fn parse_scval(b64: &str) -> Result<ScVal, String> {
    ScVal::from_xdr_base64(b64, Limits::none()).map_err(|e| format!("invalid ScVal XDR: {e}"))
}

pub fn symbol(v: &ScVal) -> Option<String> {
    match v {
        ScVal::Symbol(s) => std::str::from_utf8(s.as_slice()).ok().map(Into::into),
        _ => None,
    }
}

pub fn address(v: &ScVal) -> Option<String> {
    match v {
        ScVal::Address(a) => Some(a.to_string()),
        _ => None,
    }
}

pub fn bytes_hex(v: &ScVal) -> Option<String> {
    match v {
        ScVal::Bytes(b) => Some(hex::encode(b.as_slice())),
        _ => None,
    }
}

/// Look up a symbol-keyed field of a contract struct encoded as an `ScMap`.
pub fn map_field<'a>(v: &'a ScVal, key: &str) -> Option<&'a ScVal> {
    match v {
        ScVal::Map(Some(m)) => m
            .iter()
            .find(|e| symbol(&e.key).as_deref() == Some(key))
            .map(|e| &e.val),
        _ => None,
    }
}

fn render(v: &ScVal) -> String {
    address(v)
        .or_else(|| bytes_hex(v))
        .or_else(|| symbol(v))
        .unwrap_or_else(|| format!("{v:?}"))
}

/// Ledger sequence encoded in an RPC event id / cursor
/// (`<19-digit TOID>-<10-digit index>`, TOID = ledger << 32 | ...).
pub fn cursor_ledger(cursor: &str) -> Option<u32> {
    let toid: u64 = cursor.split('-').next()?.parse().ok()?;
    u32::try_from(toid >> 32).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::{
        AccountId, PublicKey, ScAddress, ScMap, ScMapEntry, ScSymbol, Uint256, WriteXdr,
    };

    fn addr() -> (ScVal, String) {
        let a = ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([7; 32]))));
        let s = a.to_string();
        (ScVal::Address(a), s)
    }

    fn b64(v: ScVal) -> String {
        v.to_xdr_base64(Limits::none()).unwrap()
    }

    fn sym(s: &str) -> ScVal {
        ScVal::Symbol(ScSymbol(s.try_into().unwrap()))
    }

    fn raw(topic: Vec<ScVal>, value: ScVal) -> RawEvent {
        RawEvent {
            id: "0000000429496733696-0000000000".into(),
            ledger: 100,
            ledger_closed_at: "2026-01-01T00:00:00Z".into(),
            contract_id: "C".into(),
            tx_hash: "00".into(),
            topic: topic.into_iter().map(b64).collect(),
            value: b64(value),
            in_successful_contract_call: true,
        }
    }

    #[test]
    fn decodes_attestation_recorded() {
        let (addr, addr_str) = addr();
        let data = ScVal::Map(Some(ScMap(
            vec![
                ScMapEntry {
                    key: sym("attester"),
                    val: addr,
                },
                ScMapEntry {
                    key: sym("timestamp"),
                    val: ScVal::U64(42),
                },
            ]
            .try_into()
            .unwrap(),
        )));
        let hash = ScVal::Bytes(vec![0xab; 32].try_into().unwrap());
        let ev = decode(&raw(vec![sym("attestation_recorded"), hash], data)).unwrap();
        assert_eq!(
            ev,
            LafiyaEvent::AttestationRecorded {
                record_hash: "ab".repeat(32),
                attester: addr_str,
                timestamp: 42,
            }
        );
    }

    #[test]
    fn decodes_admin_and_unknown_events() {
        let (addr, addr_str) = addr();
        let ev = decode(&raw(vec![sym("paused"), addr], ScVal::Map(None))).unwrap();
        assert_eq!(ev.name(), "paused");
        assert!(
            matches!(ev, LafiyaEvent::Admin { fields, .. } if fields["by"] == addr_str.as_str())
        );
        let ev = decode(&raw(vec![sym("something_new")], ScVal::Void)).unwrap();
        assert_eq!(
            ev,
            LafiyaEvent::Unknown {
                name: "something_new".into()
            }
        );
    }

    #[test]
    fn rejects_malformed_known_event() {
        assert!(decode(&raw(vec![sym("attester_added")], ScVal::Void)).is_err());
    }

    #[test]
    fn cursor_ledger_extracts_ledger() {
        let toid = 123u64 << 32 | 1 << 12;
        assert_eq!(cursor_ledger(&format!("{toid:019}-0000000003")), Some(123));
        assert_eq!(cursor_ledger("garbage"), None);
    }
}
