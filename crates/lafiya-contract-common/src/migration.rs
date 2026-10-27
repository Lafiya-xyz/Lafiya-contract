//! Chunked, resumable in-place data migration (cursor + state machine).
//!
//! A migration walks `total` items in index order, `step` at a time. The
//! cursor is the only migration state, so any call can be the last one and
//! the next caller resumes where it stopped. See
//! `docs/runbooks/contract-upgrade.md` ("Chunked migration").
//!
//! Read/write routing during `Running` (dual-read): item `i` lives in the
//! *new* layout iff `i < cursor`, otherwise in the *old* layout. Because the
//! cursor only advances inside `step`, routing is deterministic for any
//! interleaving of writes and steps, so no item is lost or migrated twice.

/// Hard cap on items migrated per `step`; keeps each call inside the
/// per-transaction footprint limit regardless of what the caller passes.
pub const MAX_STEP_ITEMS: u32 = 25;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Phase {
    Idle,
    Running,
    Complete,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MigrationError {
    /// `begin` called while a migration is already running.
    AlreadyRunning,
    /// `step` called while no migration is running.
    NotRunning,
}

/// Which layout holds an item right now.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Layout {
    Old,
    New,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Migration {
    phase: Phase,
    cursor: u32,
    total: u32,
}

impl Migration {
    pub const fn idle() -> Self {
        Self {
            phase: Phase::Idle,
            cursor: 0,
            total: 0,
        }
    }

    /// Start migrating `total` items (admin-gated by the caller).
    pub fn begin(&mut self, total: u32) -> Result<(), MigrationError> {
        if self.phase == Phase::Running {
            return Err(MigrationError::AlreadyRunning);
        }
        *self = Self {
            phase: if total == 0 {
                Phase::Complete
            } else {
                Phase::Running
            },
            cursor: 0,
            total,
        };
        Ok(())
    }

    /// Migrate up to `max_items` (clamped to `MAX_STEP_ITEMS`) items, calling
    /// `apply(index)` for each. Permissionless: no state beyond the cursor.
    /// Returns the number of items migrated.
    pub fn step<F: FnMut(u32)>(
        &mut self,
        max_items: u32,
        mut apply: F,
    ) -> Result<u32, MigrationError> {
        if self.phase != Phase::Running {
            return Err(MigrationError::NotRunning);
        }
        let end = self
            .cursor
            .saturating_add(max_items.min(MAX_STEP_ITEMS))
            .min(self.total);
        let done = end - self.cursor;
        for index in self.cursor..end {
            apply(index);
        }
        self.cursor = end;
        if end == self.total {
            self.phase = Phase::Complete;
        }
        Ok(done)
    }

    /// Layout holding `index` (used by both reads and writes while running).
    pub fn layout(&self, index: u32) -> Layout {
        match self.phase {
            Phase::Complete => Layout::New,
            Phase::Idle => Layout::Old,
            Phase::Running if index < self.cursor => Layout::New,
            Phase::Running => Layout::Old,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// Contracts reject non-migration state changes while this is true.
    pub fn is_paused_for_writes(&self) -> bool {
        self.phase == Phase::Running
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use std::vec::Vec;

    /// Old layout stores `v`; the migration rewrites it as `v * 2 + 1`.
    fn transform(v: u64) -> u64 {
        v * 2 + 1
    }

    #[derive(Clone, Debug)]
    enum Op {
        Step(u32),
        /// Write `value` at `index % total` through the dual-write route.
        Write(u32, u64),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0u32..40).prop_map(Op::Step),
            (any::<u32>(), 0u64..1_000).prop_map(|(i, v)| Op::Write(i, v)),
        ]
    }

    proptest! {
        /// Interrupted/resumed migration with interleaved writes equals the
        /// one-shot result, with every read consistent along the way.
        #[test]
        fn resumed_equals_one_shot(
            initial in proptest::collection::vec(0u64..1_000, 1..80),
            ops in proptest::collection::vec(op(), 0..120),
        ) {
            let total = initial.len() as u32;
            let mut old: BTreeMap<u32, u64> =
                initial.iter().cloned().enumerate().map(|(i, v)| (i as u32, v)).collect();
            let mut new: BTreeMap<u32, u64> = BTreeMap::new();
            // Reference: logical (old-layout) values after all writes.
            let mut logical: Vec<u64> = initial.clone();

            let mut m = Migration::idle();
            m.begin(total).unwrap();

            let read = |m: &Migration, old: &BTreeMap<u32, u64>, new: &BTreeMap<u32, u64>, i: u32| {
                match m.layout(i) {
                    Layout::New => new.get(&i).copied(),
                    Layout::Old => old.get(&i).copied().map(transform),
                }
            };

            for op in ops {
                match op {
                    Op::Step(n) => {
                        if m.phase() == Phase::Running {
                            let mut moved = Vec::new();
                            m.step(n, |i| moved.push(i)).unwrap();
                            for i in moved {
                                // move: no loss, no duplication
                                let v = old.remove(&i).unwrap();
                                prop_assert!(new.insert(i, transform(v)).is_none());
                            }
                        }
                    }
                    Op::Write(i, v) => {
                        let i = i % total;
                        match m.layout(i) {
                            Layout::New => { new.insert(i, transform(v)); }
                            Layout::Old => { old.insert(i, v); }
                        }
                        logical[i as usize] = v;
                    }
                }
                for i in 0..total {
                    prop_assert_eq!(read(&m, &old, &new, i), Some(transform(logical[i as usize])));
                }
            }

            // Drain to completion.
            while m.phase() == Phase::Running {
                let mut moved = Vec::new();
                m.step(MAX_STEP_ITEMS, |i| moved.push(i)).unwrap();
                for i in moved {
                    let v = old.remove(&i).unwrap();
                    new.insert(i, transform(v));
                }
            }
            let one_shot: BTreeMap<u32, u64> = logical
                .iter()
                .enumerate()
                .map(|(i, v)| (i as u32, transform(*v)))
                .collect();
            prop_assert!(old.is_empty());
            prop_assert_eq!(new, one_shot);
        }
    }

    #[test]
    fn state_machine_and_bounds() {
        let mut m = Migration::idle();
        assert_eq!(m.step(1, |_| {}), Err(MigrationError::NotRunning));
        m.begin(60).unwrap();
        assert!(m.is_paused_for_writes());
        assert_eq!(m.begin(1), Err(MigrationError::AlreadyRunning));
        assert_eq!(m.step(u32::MAX, |_| {}).unwrap(), MAX_STEP_ITEMS);
        assert_eq!(m.cursor(), MAX_STEP_ITEMS);
        while m.phase() == Phase::Running {
            m.step(MAX_STEP_ITEMS, |_| {}).unwrap();
        }
        assert_eq!(m.cursor(), 60);
        assert!(!m.is_paused_for_writes());
        m.begin(0).unwrap();
        assert_eq!(m.phase(), Phase::Complete);
    }
}
