import type { NetworkTrust } from "./trust.js";

/** An attestation as stored by attestation-registry. */
export interface OnChainAttestation {
  attester: string;
  /** Ledger timestamp (unix seconds) the attestation was recorded at. */
  timestamp: bigint;
}

/** attester-registry's view of an attester (`get_attester_status`). */
export interface AttesterState {
  suspended: boolean;
  region?: string;
}

/**
 * The reads verification needs. {@link BindingsChainReader} implements it
 * over Soroban RPC; {@link OfflineChainReader} over a signed-off snapshot;
 * tests use fakes.
 */
export interface ChainReader {
  readonly source: "rpc" | "offline";
  getAttestation(trust: NetworkTrust, recordHash: string): Promise<OnChainAttestation | null>;
  /** Oldest first, as `get_attestation_history` returns it. */
  getAttestationHistory(trust: NetworkTrust, recordHash: string): Promise<OnChainAttestation[]>;
  /** `null` when the attester is not (or no longer) on the allowlist. */
  getAttesterStatus(trust: NetworkTrust, attester: string): Promise<AttesterState | null>;
  /**
   * Whether `recordHash` was explicitly revoked. `revoke_attestation`
   * deletes storage, so on-chain reads alone cannot tell "revoked" from
   * "never attested"; a reader backed by events or an indexer can.
   */
  isRevoked?(trust: NetworkTrust, recordHash: string): Promise<boolean>;
}

/** A snapshot of the chain state for offline verification. */
export interface OfflineBundle {
  network: string;
  /** Unix seconds the snapshot was taken at. */
  snapshotAt: number;
  /** Record hash -> attestation history, oldest first. */
  attestations: Record<string, { attester: string; timestamp: string }[]>;
  /** Allowlisted attesters and their status. */
  attesters: Record<string, AttesterState>;
  /** Record hashes revoked by the admin. */
  revoked: string[];
}

/** Verify from an {@link OfflineBundle} (no network access). */
export class OfflineChainReader implements ChainReader {
  readonly source = "offline" as const;

  constructor(readonly bundle: OfflineBundle) {}

  async getAttestationHistory(_t: NetworkTrust, recordHash: string) {
    return (this.bundle.attestations[recordHash] ?? []).map((a) => ({
      attester: a.attester,
      timestamp: BigInt(a.timestamp),
    }));
  }

  async getAttestation(t: NetworkTrust, recordHash: string) {
    return (await this.getAttestationHistory(t, recordHash)).at(-1) ?? null;
  }

  async getAttesterStatus(_t: NetworkTrust, attester: string) {
    return this.bundle.attesters[attester] ?? null;
  }

  async isRevoked(_t: NetworkTrust, recordHash: string) {
    return this.bundle.revoked.includes(recordHash);
  }
}
