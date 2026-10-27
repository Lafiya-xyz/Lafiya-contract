import type { CallOptions } from "./transport.js";
import type { NetworkTrust } from "./trust.js";
import {
  type StatusListSegment,
  type StatusListRoot,
  verifySegmentInclusion,
  isRevokedInSegment,
} from "./status-list.js";

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
 *
 * Every read takes optional {@link CallOptions} (`signal`, `timeoutMs`);
 * implementations that do I/O must honour them and should reject with a
 * `TransportError`.
 */
export interface ChainReader {
  readonly source: "rpc" | "offline";
  /** Unix seconds the data was captured at; set by snapshot-backed readers. */
  readonly snapshotAt?: number;
  getAttestation(trust: NetworkTrust, recordHash: string): Promise<OnChainAttestation | null>;
  /** Oldest first, as `get_attestation_history` returns it. */
  getAttestationHistory(
    trust: NetworkTrust,
    recordHash: string,
    opts?: CallOptions,
  ): Promise<OnChainAttestation[]>;
  /** `null` when the attester is not (or no longer) on the allowlist. */
  getAttesterStatus(
    trust: NetworkTrust,
    attester: string,
    opts?: CallOptions,
  ): Promise<AttesterState | null>;
  /**
   * Whether `recordHash` was explicitly revoked. `revoke_attestation`
   * deletes storage, so on-chain reads alone cannot tell "revoked" from
   * "never attested"; a reader backed by events or an indexer can.
   */
  isRevoked?(trust: NetworkTrust, recordHash: string): Promise<boolean>;
  /**
   * Fetch a status-list segment by index. Implementations backed by RPC
   * read it from the status-list contract; offline readers serve it from
   * their cached segments.
   */
  getStatusListSegment?(trust: NetworkTrust, segmentIndex: number): Promise<StatusListSegment | null>;
  /** The signed root the status-list contract currently commits to. */
  getStatusListRoot?(trust: NetworkTrust): Promise<StatusListRoot | null>;
}

/** A status-list segment cached for offline verification. */
export interface CachedStatusListSegment {
  segment: StatusListSegment;
  /** Unix seconds the segment was fetched at. */
  fetchedAt: number;
}

/**
 * Syncs status-list segments from a {@link ChainReader} and caches them,
 * validating each segment's inclusion against the signed root before it is
 * trusted. Cached segments let offline verifiers answer revocation checks
 * without per-record RPC.
 */
export class StatusListCache {
  private readonly segments = new Map<number, CachedStatusListSegment>();

  constructor(private readonly reader: ChainReader) {}

  /**
   * Fetch and validate the segment at `segmentIndex`. Returns `null` when
   * the reader cannot serve status lists or the segment is missing. Throws
   * if the segment does not prove inclusion under the signed root.
   */
  async syncSegment(trust: NetworkTrust, segmentIndex: number): Promise<StatusListSegment | null> {
    if (!this.reader.getStatusListSegment || !this.reader.getStatusListRoot) return null;
    const [segment, root] = await Promise.all([
      this.reader.getStatusListSegment(trust, segmentIndex),
      this.reader.getStatusListRoot(trust),
    ]);
    if (!segment || !root) return null;
    if (!verifySegmentInclusion(root, segment)) {
      throw new Error(`status-list segment ${segmentIndex} failed inclusion check`);
    }
    this.segments.set(segmentIndex, { segment, fetchedAt: Date.now() / 1000 });
    return segment;
  }

  /** Sync every segment index listed in the signed root. */
  async syncAll(trust: NetworkTrust): Promise<number> {
    if (!this.reader.getStatusListRoot) return 0;
    const root = await this.reader.getStatusListRoot(trust);
    if (!root) return 0;
    let synced = 0;
    for (const segmentIndex of root.segmentIndexes) {
      if (await this.syncSegment(trust, segmentIndex)) synced++;
    }
    return synced;
  }

  /** A cached segment, or `null` if it has not been synced. */
  get(segmentIndex: number): StatusListSegment | null {
    return this.segments.get(segmentIndex)?.segment ?? null;
  }

  /** Whether `recordHash` is revoked according to the cached segments. */
  isRevoked(recordHash: string): boolean {
    for (const { segment } of this.segments.values()) {
      if (isRevokedInSegment(segment, recordHash)) return true;
    }
    return false;
  }

  /** Drop all cached segments (e.g. before a fresh sync). */
  clear(): void {
    this.segments.clear();
  }
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
  /** Status-list segments captured in the snapshot, keyed by segment index. */
  statusListSegments?: Record<string, StatusListSegment>;
  /** The signed status-list root the captured segments commit to. */
  statusListRoot?: StatusListRoot;
}

/** Verify from an {@link OfflineBundle} (no network access). */
export class OfflineChainReader implements ChainReader {
  readonly source = "offline" as const;

  constructor(readonly bundle: OfflineBundle) {}

  get snapshotAt(): number {
    return this.bundle.snapshotAt;
  }

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
    if (this.bundle.revoked.includes(recordHash)) return true;
    for (const segment of Object.values(this.bundle.statusListSegments ?? {})) {
      if (isRevokedInSegment(segment, recordHash)) return true;
    }
    return false;
  }

  async getStatusListSegment(_t: NetworkTrust, segmentIndex: number) {
    return this.bundle.statusListSegments?.[String(segmentIndex)] ?? null;
  }

  async getStatusListRoot(_t: NetworkTrust) {
    return this.bundle.statusListRoot ?? null;
  }
}
