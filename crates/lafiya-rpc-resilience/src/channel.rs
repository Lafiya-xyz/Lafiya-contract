//! Channel-account pool for concurrent submissions (issue #748).
//!
//! A single sponsor account serialises every submission: two transactions
//! built from the same account at the same time share a sequence number, so
//! one of them is rejected with `txBadSeq`. The pool leases a *channel*
//! account per in-flight submission instead, so concurrent submissions never
//! contend on a sequence number.
//!
//! Each channel tracks its next sequence number locally. The local value is
//! authoritative only while the channel is leased; on `txBadSeq` the channel
//! re-syncs from RPC (`account_sequence`), and on an ambiguous outcome
//! (timeout / lost response) the channel is **quarantined** until the pending
//! transaction is resolved by hash -- reusing it earlier would risk a
//! duplicate submission on the same sequence number.
//!
//! The pool is transport-agnostic: it hands out [`ChannelLease`]s and records
//! outcomes; the caller builds and submits the transaction. [`ChannelPool::submit`]
//! wires the two together for the common case.

use crate::{RpcError, RpcProvider, SignedTx, SubmitOutcome, TxState};
use std::collections::HashMap;

/// One channel account and its locally tracked sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    /// Channel account address (`G...`).
    pub account: String,
    /// Sequence number the *next* transaction from this account must use.
    pub next_sequence: i64,
    /// Set while a transaction from this channel has an unresolved outcome.
    pub quarantine: Option<Quarantine>,
}

/// Why a channel is out of rotation, and what resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quarantine {
    /// Hash of the transaction whose fate is unknown.
    pub tx_hash: String,
    /// Sequence number that transaction consumed (if it landed).
    pub sequence: i64,
    /// The RPC-level error that made the outcome ambiguous.
    pub error: RpcError,
}

/// A leased channel. Dropping it without [`ChannelPool::release`] leaves the
/// channel leased (the caller must always release, even on error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelLease {
    pub account: String,
    /// Sequence number reserved for this submission.
    pub sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolError {
    /// Every channel is leased or quarantined.
    Exhausted,
    /// The provider could not report the channel's sequence number.
    SequenceUnavailable(RpcError),
    /// The channel is quarantined; resolve the pending transaction first.
    Quarantined { account: String, tx_hash: String },
    /// The provider refused the submission before it reached the network.
    SubmitFailed(RpcError),
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PoolError::Exhausted => write!(f, "no channel account available"),
            PoolError::SequenceUnavailable(e) => write!(f, "sequence unavailable: {e}"),
            PoolError::Quarantined { account, tx_hash } => {
                write!(f, "channel {account} quarantined by {tx_hash}")
            }
            PoolError::SubmitFailed(e) => write!(f, "submit failed: {e}"),
        }
    }
}

impl std::error::Error for PoolError {}

/// Result of one pooled submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PooledOutcome {
    pub account: String,
    pub sequence: i64,
    pub outcome: SubmitOutcome,
}

/// A pool of channel accounts with locally tracked sequence numbers.
pub struct ChannelPool {
    channels: Vec<Channel>,
    /// account -> index into `channels`, for O(1) lookup on release.
    index: HashMap<String, usize>,
    /// Accounts currently leased to a caller.
    leased: Vec<bool>,
}

impl ChannelPool {
    /// Build a pool from channel accounts whose sequence numbers are already
    /// known (e.g. loaded from the relayer's persisted state).
    pub fn new(channels: impl IntoIterator<Item = (String, i64)>) -> Self {
        let mut out = Vec::new();
        let mut index = HashMap::new();
        for (account, next_sequence) in channels {
            index.insert(account.clone(), out.len());
            out.push(Channel {
                account,
                next_sequence,
                quarantine: None,
            });
        }
        let leased = vec![false; out.len()];
        ChannelPool {
            channels: out,
            index,
            leased,
        }
    }

    /// Build a pool by reading each channel's sequence number from RPC.
    pub fn from_provider<P: RpcProvider>(
        accounts: &[String],
        provider: &mut P,
    ) -> Result<Self, PoolError> {
        let mut channels = Vec::with_capacity(accounts.len());
        for account in accounts {
            let seq = provider
                .account_sequence(account)
                .map_err(PoolError::SequenceUnavailable)?;
            channels.push((account.clone(), seq));
        }
        Ok(Self::new(channels))
    }

    pub fn len(&self) -> usize {
        self.channels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Number of channels that can be leased right now.
    pub fn available(&self) -> usize {
        self.channels
            .iter()
            .enumerate()
            .filter(|(i, c)| !self.leased[*i] && c.quarantine.is_none())
            .count()
    }

    /// Channels currently quarantined, with the transaction that blocked them.
    pub fn quarantined(&self) -> Vec<(&str, &Quarantine)> {
        self.channels
            .iter()
            .filter_map(|c| c.quarantine.as_ref().map(|q| (c.account.as_str(), q)))
            .collect()
    }

    /// Lease a free channel and reserve its next sequence number.
    pub fn lease(&mut self) -> Result<ChannelLease, PoolError> {
        let idx = self
            .channels
            .iter()
            .enumerate()
            .position(|(i, c)| !self.leased[i] && c.quarantine.is_none())
            .ok_or(PoolError::Exhausted)?;
        self.leased[idx] = true;
        let channel = &mut self.channels[idx];
        let sequence = channel.next_sequence;
        channel.next_sequence += 1;
        Ok(ChannelLease {
            account: channel.account.clone(),
            sequence,
        })
    }

    /// Return a channel after a definite outcome. `consumed` says whether the
    /// reserved sequence number was used on-chain; if not, the local counter
    /// is rolled back so the number is not skipped.
    pub fn release(&mut self, lease: &ChannelLease, consumed: bool) {
        let Some(&idx) = self.index.get(&lease.account) else {
            return;
        };
        self.leased[idx] = false;
        if !consumed {
            let channel = &mut self.channels[idx];
            if channel.next_sequence == lease.sequence + 1 {
                channel.next_sequence = lease.sequence;
            }
        }
    }

    /// Re-sync a channel's sequence number from RPC after `txBadSeq`.
    pub fn resync<P: RpcProvider>(
        &mut self,
        account: &str,
        provider: &mut P,
    ) -> Result<i64, PoolError> {
        let seq = provider
            .account_sequence(account)
            .map_err(PoolError::SequenceUnavailable)?;
        if let Some(&idx) = self.index.get(account) {
            self.channels[idx].next_sequence = seq;
        }
        Ok(seq)
    }

    /// Quarantine a channel whose transaction outcome is unknown. The channel
    /// stays out of rotation until [`ChannelPool::resolve`] is called.
    pub fn quarantine(&mut self, lease: &ChannelLease, tx_hash: &str, error: RpcError) {
        let Some(&idx) = self.index.get(&lease.account) else {
            return;
        };
        self.leased[idx] = false;
        self.channels[idx].quarantine = Some(Quarantine {
            tx_hash: tx_hash.to_string(),
            sequence: lease.sequence,
            error,
        });
    }

    /// Resolve a quarantined channel by polling the pending transaction.
    /// Returns `true` once the channel is back in rotation; `false` while the
    /// transaction is still unresolved (the channel stays quarantined).
    pub fn resolve<P: RpcProvider>(
        &mut self,
        account: &str,
        provider: &mut P,
    ) -> Result<bool, PoolError> {
        let Some(&idx) = self.index.get(account) else {
            return Ok(false);
        };
        let Some(q) = self.channels[idx].quarantine.clone() else {
            return Ok(true);
        };
        match provider.get_transaction(&q.tx_hash) {
            Ok(TxState::Accepted { .. }) | Ok(TxState::Rejected { .. }) => {
                // The sequence number is consumed either way.
                let channel = &mut self.channels[idx];
                channel.quarantine = None;
                if channel.next_sequence <= q.sequence {
                    channel.next_sequence = q.sequence + 1;
                }
                Ok(true)
            }
            Ok(TxState::Expired { .. }) => {
                // Never includable: the sequence number was not consumed.
                let channel = &mut self.channels[idx];
                channel.quarantine = None;
                if channel.next_sequence == q.sequence + 1 {
                    channel.next_sequence = q.sequence;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Lease a channel, submit `tx`, and record the outcome. On `txBadSeq`
    /// the channel re-syncs from RPC and the submission is retried once on
    /// the same channel; on an ambiguous outcome the channel is quarantined.
    pub fn submit<P: RpcProvider>(
        &mut self,
        provider: &mut P,
        tx: &SignedTx,
    ) -> Result<PooledOutcome, PoolError> {
        let lease = self.lease()?;
        let mut outcome = provider.submit(tx);
        if is_bad_seq(&outcome) {
            self.resync(&lease.account, provider)?;
            outcome = provider.submit(tx);
        }
        match &outcome {
            SubmitOutcome::Definite(e) => {
                self.release(&lease, false);
                Err(PoolError::SubmitFailed(e.clone()))
            }
            SubmitOutcome::Ambiguous(e) => {
                self.quarantine(&lease, &tx.hash, e.clone());
                Ok(PooledOutcome {
                    account: lease.account,
                    sequence: lease.sequence,
                    outcome,
                })
            }
            SubmitOutcome::Ack(_) => {
                self.release(&lease, true);
                Ok(PooledOutcome {
                    account: lease.account,
                    sequence: lease.sequence,
                    outcome,
                })
            }
        }
    }
}

/// `txBadSeq` surfaces as a definite rejection carrying the RPC message.
fn is_bad_seq(outcome: &SubmitOutcome) -> bool {
    match outcome {
        SubmitOutcome::Ack(TxState::Rejected { reason }) => {
            let r = reason.to_ascii_lowercase();
            r.contains("txbadseq") || r.contains("bad seq")
        }
        _ => false,
    }
}
