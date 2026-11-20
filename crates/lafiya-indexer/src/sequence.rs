//! Gap detection from the RPC event ordering key.
//!
//! Registry contracts no longer maintain a shared, globally incrementing
//! `seq` counter: writing one on every event serialized all attestations
//! against each other and rewrote the whole instance entry. Instead, events
//! are ordered by the `(ledger, tx index, event index)` triple that RPC
//! already provides (the event id is `"{ledger}-{tx_index}-{event_index}"`).
//!
//! Gap detection therefore works on that ordering key: within a contract, the
//! key must strictly increase in chain order. A non-increasing key means the
//! provider returned events out of order or duplicated them; a jump in the
//! ledger/tx/event triple is *not* by itself a gap, because other contracts'
//! events (and failed calls) legitimately occupy the intervening positions.
//! Ledger-level completeness is inferred separately from ledger ranges.

use std::collections::HashMap;

use crate::decode::RawEvent;

/// A hole or ordering violation in a contract's event stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqGap {
    pub contract_id: String,
    /// The ordering key that should have come next (strictly greater than the
    /// previously seen key).
    pub expected: EventKey,
    /// The ordering key actually seen.
    pub got: EventKey,
}

/// The RPC-provided ordering key for an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventKey {
    pub ledger: u32,
    pub tx_index: u32,
    pub event_index: u32,
}

impl EventKey {
    /// Parse an RPC event id of the form `"{ledger}-{tx_index}-{event_index}"`.
    ///
    /// RPC zero-pads each component, but parsing is tolerant of unpadded input.
    pub fn from_event_id(id: &str) -> Option<Self> {
        let mut parts = id.split('-');
        let ledger = parts.next()?.parse().ok()?;
        let tx_index = parts.next()?.parse().ok()?;
        let event_index = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(EventKey {
            ledger,
            tx_index,
            event_index,
        })
    }
}

/// The ordering key of a raw event, if its id is well formed.
pub fn event_key(raw: &RawEvent) -> Option<EventKey> {
    EventKey::from_event_id(&raw.id)
}

/// Tracks the last ordering key seen per contract, in memory. The first event
/// observed for a contract after (re)start only seeds the tracker.
#[derive(Debug, Default)]
pub struct SeqTracker {
    last: HashMap<String, EventKey>,
}

impl SeqTracker {
    /// Check a page of events (in chain order) and report every ordering
    /// violation. Events with an unparseable id are ignored.
    pub fn observe(&mut self, events: &[RawEvent]) -> Vec<SeqGap> {
        let mut gaps = Vec::new();
        for e in events.iter().filter(|e| e.in_successful_contract_call) {
            let Some(got) = event_key(e) else { continue };
            if let Some(prev) = self.last.insert(e.contract_id.clone(), got) {
                if got <= prev {
                    gaps.push(SeqGap {
                        contract_id: e.contract_id.clone(),
                        expected: EventKey {
                            ledger: prev.ledger,
                            tx_index: prev.tx_index,
                            event_index: prev.event_index.saturating_add(1),
                        },
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
    use stellar_xdr::{Limits, ScMap, WriteXdr};

    fn ev(contract: &str, id: &str) -> RawEvent {
        RawEvent {
            id: id.into(),
            ledger: 100,
            ledger_closed_at: "2026-01-01T00:00:00Z".into(),
            contract_id: contract.into(),
            tx_hash: "00".into(),
            topic: Vec::new(),
            value: ScVal::Map(Some(ScMap(Vec::new().try_into().unwrap())))
                .to_xdr_base64(Limits::none())
                .unwrap(),
            in_successful_contract_call: true,
        }
    }

    #[test]
    fn event_key_parses_padded_and_unpadded_ids() {
        assert_eq!(
            EventKey::from_event_id("0000000429496733696-0000000000-0000000001"),
            Some(EventKey {
                ledger: 429496733696,
                tx_index: 0,
                event_index: 1
            })
        );
        assert_eq!(
            EventKey::from_event_id("7-2-3"),
            Some(EventKey {
                ledger: 7,
                tx_index: 2,
                event_index: 3
            })
        );
        assert_eq!(EventKey::from_event_id("7-2"), None);
        assert_eq!(EventKey::from_event_id("7-2-3-4"), None);
        assert_eq!(EventKey::from_event_id("nope"), None);
    }

    #[test]
    fn increasing_keys_raise_no_gap() {
        let mut t = SeqTracker::default();
        let page = [ev("A", "100-0-0"), ev("A", "100-0-1"), ev("B", "100-1-0")];
        assert!(t.observe(&page).is_empty());
        assert!(t.observe(&[ev("A", "101-0-0"), ev("B", "101-0-1")]).is_empty());
    }

    #[test]
    fn sparse_keys_are_not_gaps() {
        // Other contracts' events and failed calls occupy the intervening
        // positions, so a jump in the triple is legitimate.
        let mut t = SeqTracker::default();
        assert!(t
            .observe(&[ev("A", "100-0-0"), ev("A", "100-5-9")])
            .is_empty());
    }

    #[test]
    fn out_of_order_event_within_a_page_is_detected() {
        let mut t = SeqTracker::default();
        let gaps = t.observe(&[ev("A", "100-0-0"), ev("A", "100-0-2"), ev("A", "100-0-1")]);
        assert_eq!(
            gaps,
            vec![SeqGap {
                contract_id: "A".into(),
                expected: EventKey {
                    ledger: 100,
                    tx_index: 0,
                    event_index: 3
                },
                got: EventKey {
                    ledger: 100,
                    tx_index: 0,
                    event_index: 1
                },
            }]
        );
    }

    #[test]
    fn duplicate_event_across_pages_is_detected() {
        let mut t = SeqTracker::default();
        assert!(t
            .observe(&[ev("A", "100-0-0"), ev("A", "100-0-1")])
            .is_empty());
        assert_eq!(t.observe(&[ev("A", "100-0-1")]).len(), 1);
    }

    #[test]
    fn events_with_unparseable_ids_are_ignored() {
        let mut t = SeqTracker::default();
        assert!(t
            .observe(&[
                ev("A", "garbage"),
                ev("A", "100-0-0"),
                ev("A", "also-garbage"),
                ev("A", "100-0-1"),
            ])
            .is_empty());
    }
}
