//! Gap detection from per-contract event sequence numbers.
//!
//! Registry events carry a monotonic `seq` data field (1, 2, 3, ...) that
//! advances with every emitted event. A jump means the RPC provider dropped
//! events, independent of ledger-based completeness inference. Events without
//! a `seq` (older schema, or contracts that do not emit one) are ignored.

use std::collections::HashMap;

use crate::decode::{map_field, parse_scval, RawEvent};
use stellar_xdr::ScVal;

/// A hole in a contract's event sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqGap {
    pub contract_id: String,
    /// The sequence number that should have come next.
    pub expected: u64,
    /// The sequence number actually seen.
    pub got: u64,
}

/// The `seq` field of a raw event, if present.
pub fn event_seq(raw: &RawEvent) -> Option<u64> {
    match map_field(&parse_scval(&raw.value).ok()?, "seq")? {
        ScVal::U64(n) => Some(*n),
        _ => None,
    }
}

/// Tracks the last sequence number seen per contract, in memory. The first
/// event observed for a contract after (re)start only seeds the tracker.
#[derive(Debug, Default)]
pub struct SeqTracker {
    last: HashMap<String, u64>,
}

impl SeqTracker {
    /// Check a page of events (in chain order) and report every gap.
    pub fn observe(&mut self, events: &[RawEvent]) -> Vec<SeqGap> {
        let mut gaps = Vec::new();
        for e in events.iter().filter(|e| e.in_successful_contract_call) {
            let Some(got) = event_seq(e) else { continue };
            if let Some(prev) = self.last.insert(e.contract_id.clone(), got) {
                if got != prev + 1 {
                    gaps.push(SeqGap {
                        contract_id: e.contract_id.clone(),
                        expected: prev + 1,
                        got,
                    });
                }
            }
        }
        gaps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::{Limits, ScMap, ScMapEntry, ScSymbol, WriteXdr};

    fn ev(contract: &str, seq: Option<u64>) -> RawEvent {
        let entries = seq
            .map(|n| ScMapEntry {
                key: ScVal::Symbol(ScSymbol("seq".try_into().unwrap())),
                val: ScVal::U64(n),
            })
            .into_iter()
            .collect::<Vec<_>>();
        RawEvent {
            id: "0000000429496733696-0000000000".into(),
            ledger: 100,
            ledger_closed_at: "2026-01-01T00:00:00Z".into(),
            contract_id: contract.into(),
            tx_hash: "00".into(),
            topic: Vec::new(),
            value: ScVal::Map(Some(ScMap(entries.try_into().unwrap())))
                .to_xdr_base64(Limits::none())
                .unwrap(),
            in_successful_contract_call: true,
        }
    }

    #[test]
    fn contiguous_events_raise_no_gap() {
        let mut t = SeqTracker::default();
        let page = [ev("A", Some(5)), ev("A", Some(6)), ev("B", Some(1))];
        assert!(t.observe(&page).is_empty());
        assert!(t.observe(&[ev("A", Some(7)), ev("B", Some(2))]).is_empty());
    }

    #[test]
    fn dropped_event_within_a_page_is_detected() {
        let mut t = SeqTracker::default();
        let gaps = t.observe(&[ev("A", Some(1)), ev("A", Some(2)), ev("A", Some(4))]);
        assert_eq!(
            gaps,
            vec![SeqGap {
                contract_id: "A".into(),
                expected: 3,
                got: 4
            }]
        );
    }

    #[test]
    fn dropped_event_across_pages_is_detected() {
        let mut t = SeqTracker::default();
        assert!(t.observe(&[ev("A", Some(1)), ev("A", Some(2))]).is_empty());
        assert_eq!(t.observe(&[ev("A", Some(5))]).len(), 1);
    }

    #[test]
    fn events_without_seq_are_ignored() {
        let mut t = SeqTracker::default();
        assert!(t
            .observe(&[ev("A", None), ev("A", Some(1)), ev("A", None), ev("A", Some(2))])
            .is_empty());
    }
}
