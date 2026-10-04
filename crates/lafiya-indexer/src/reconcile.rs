//! Periodic reconciliation of derived state against on-chain storage.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use stellar_xdr::{ScAddress, ScVal};

use crate::decode::{address, bytes_hex, map_field, symbol};
use crate::metrics::Metrics;
use crate::rpc::{RpcClient, RpcError};

/// `get_attester_status` result (`None` when not allowlisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainAttester {
    pub suspended: bool,
    pub region: Option<String>,
    pub license_hash: Option<String>,
}

/// `get_attestation` result: the current attester and timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainAttestation {
    pub attester: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DerivedAttester {
    pub address: String,
    pub allowlisted: bool,
    pub suspended: bool,
    pub region: Option<String>,
    pub license_hash: Option<String>,
    pub info_stale: bool,
}

pub fn parse_attester_status(v: &ScVal) -> Option<ChainAttester> {
    let info = map_field(v, "info")?;
    Some(ChainAttester {
        suspended: matches!(map_field(v, "suspended"), Some(ScVal::Bool(true))),
        region: map_field(info, "region").and_then(symbol),
        license_hash: map_field(info, "license_hash").and_then(bytes_hex),
    })
}

pub fn parse_attestation(v: &ScVal) -> Option<ChainAttestation> {
    Some(ChainAttestation {
        attester: map_field(v, "attester").and_then(address)?,
        timestamp: match map_field(v, "timestamp")? {
            ScVal::U64(t) => *t,
            _ => return None,
        },
    })
}

pub fn attester_matches(derived: &DerivedAttester, chain: Option<&ChainAttester>) -> bool {
    match chain {
        None => !derived.allowlisted,
        Some(c) => {
            derived.allowlisted
                && derived.suspended == c.suspended
                && (derived.info_stale
                    || (derived.region == c.region && derived.license_hash == c.license_hash))
        }
    }
}

pub struct Reconciler {
    pub pool: PgPool,
    pub rpc: RpcClient,
    pub metrics: Arc<Metrics>,
    pub attester_registry: String,
    pub attestation_registry: String,
    pub sample_size: i64,
    /// Skip a cycle while ingestion lags more than this, to avoid reporting
    /// changes that are simply not ingested yet.
    pub max_lag_ledgers: i64,
}

impl Reconciler {
    pub async fn chain_attester(&self, addr: &str) -> Result<Option<ChainAttester>, RpcError> {
        let arg = ScVal::Address(
            addr.parse::<ScAddress>()
                .map_err(|e| RpcError::Response(format!("address {addr}: {e}")))?,
        );
        let v = self
            .rpc
            .read_contract(&self.attester_registry, "get_attester_status", vec![arg])
            .await?;
        Ok(parse_attester_status(&v))
    }

    pub async fn chain_attestation(
        &self,
        hash: &str,
    ) -> Result<Option<ChainAttestation>, RpcError> {
        let bytes = hex::decode(hash).map_err(|e| RpcError::Response(e.to_string()))?;
        let arg = ScVal::Bytes(
            bytes
                .try_into()
                .map_err(|_| RpcError::Response("hash".into()))?,
        );
        let v = self
            .rpc
            .read_contract(&self.attestation_registry, "get_attestation", vec![arg])
            .await?;
        Ok(parse_attestation(&v))
    }

    /// Fill in region / license hash for attesters whose info changed.
    async fn refresh_stale_info(&self) -> anyhow::Result<()> {
        let stale: Vec<(String,)> =
            sqlx::query_as("SELECT address FROM attesters WHERE info_stale LIMIT 100")
                .fetch_all(&self.pool)
                .await?;
        for (addr,) in stale {
            let chain = self.chain_attester(&addr).await?;
            sqlx::query(
                "UPDATE attesters SET region = $2, license_hash = $3, info_stale = false
                 WHERE address = $1 AND info_stale",
            )
            .bind(&addr)
            .bind(chain.as_ref().and_then(|c| c.region.clone()))
            .bind(chain.as_ref().and_then(|c| c.license_hash.clone()))
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Run one reconciliation cycle; returns the number of mismatches.
    pub async fn cycle(&self) -> anyhow::Result<u64> {
        self.refresh_stale_info().await?;
        if self.metrics.lag_ledgers.get() > self.max_lag_ledgers {
            tracing::info!("ingestion lagging; skipping reconciliation sample");
            return Ok(0);
        }
        let mut mismatches = 0;

        let attesters: Vec<DerivedAttester> = sqlx::query_as(
            "SELECT address, allowlisted, suspended, region, license_hash, info_stale
             FROM attesters ORDER BY random() LIMIT $1",
        )
        .bind(self.sample_size)
        .fetch_all(&self.pool)
        .await?;
        for a in attesters {
            let chain = self.chain_attester(&a.address).await?;
            self.metrics
                .reconcile_checks_total
                .with_label_values(&["attester"])
                .inc();
            if !attester_matches(&a, chain.as_ref()) {
                mismatches += 1;
                self.metrics
                    .reconcile_mismatches_total
                    .with_label_values(&["attester"])
                    .inc();
                tracing::warn!(attester = %a.address, derived = ?a, ?chain, "attester mismatch");
            }
        }

        let records: Vec<(String, Option<String>, Option<i64>)> = sqlx::query_as(
            "SELECT r.record_hash, cur.attester, cur.attested_at
             FROM (SELECT DISTINCT record_hash FROM attestations) r
             LEFT JOIN LATERAL (
                 SELECT attester, attested_at FROM attestations a
                 WHERE a.record_hash = r.record_hash AND a.revoked_event_id IS NULL
                 ORDER BY event_id DESC LIMIT 1
             ) cur ON true
             ORDER BY random() LIMIT $1",
        )
        .bind(self.sample_size)
        .fetch_all(&self.pool)
        .await?;
        for (hash, attester, attested_at) in records {
            let derived = attester
                .zip(attested_at)
                .map(|(attester, t)| ChainAttestation {
                    attester,
                    timestamp: t as u64,
                });
            let chain = self.chain_attestation(&hash).await?;
            self.metrics
                .reconcile_checks_total
                .with_label_values(&["attestation"])
                .inc();
            if derived != chain {
                mismatches += 1;
                self.metrics
                    .reconcile_mismatches_total
                    .with_label_values(&["attestation"])
                    .inc();
                tracing::warn!(record_hash = %hash, ?derived, ?chain, "attestation mismatch");
            }
        }
        Ok(mismatches)
    }

    pub async fn run(&self, interval: Duration) {
        loop {
            if let Err(e) = self.cycle().await {
                tracing::warn!(error = %e, "reconciliation cycle failed");
            }
            tokio::time::sleep(interval).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn derived(allowlisted: bool, suspended: bool) -> DerivedAttester {
        DerivedAttester {
            address: "G".into(),
            allowlisted,
            suspended,
            region: Some("lagos".into()),
            license_hash: None,
            info_stale: false,
        }
    }

    #[test]
    fn attester_comparison() {
        let chain = ChainAttester {
            suspended: false,
            region: Some("lagos".into()),
            license_hash: None,
        };
        assert!(attester_matches(&derived(true, false), Some(&chain)));
        assert!(!attester_matches(&derived(true, true), Some(&chain)));
        assert!(!attester_matches(&derived(false, false), Some(&chain)));
        assert!(attester_matches(&derived(false, false), None));
        assert!(!attester_matches(&derived(true, false), None));
        let moved = ChainAttester {
            region: Some("kano".into()),
            ..chain
        };
        assert!(!attester_matches(&derived(true, false), Some(&moved)));
        let stale = DerivedAttester {
            info_stale: true,
            ..derived(true, false)
        };
        assert!(attester_matches(&stale, Some(&moved)));
    }
}
