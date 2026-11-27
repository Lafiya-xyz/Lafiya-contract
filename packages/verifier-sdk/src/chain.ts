import type { CallOptions } from "./transport.js";
import type { NetworkTrust } from "./trust.js";
import {
  type StatusListSegment,
  type StatusListRoot,
  type StatusListEpochRoot,
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
   * Fetch a status-list segment by index for a specific epoch. Implementations
   * backed by RPC read it from the status-list contract; offline readers serve
   * it from their cached segments. Segments are always read for the epoch of
   * the root the verifier trusts, never for an implicit "current" epoch.
   */
  getStatusListSegment?(
    trust: NetworkTrust,
    epoch: bigint,
    segmentIndex: number,
  ): Promise<StatusListSegment | null>;
  /**
   * The signed root the status-list contract commits to for `epoch`. Verifiers
   * must read this first and then fetch segments for the same epoch.
   */
  getStatusListRoot?(trust: NetworkTrust, epoch: bigint): Promise<StatusListRoot | null>;
  /**
   * The newest epoch the status-list contract has an activated root for, or
   * `null` when no root has been published yet.
   */
  getStatusListEpoch?(trust: NetworkTrust): Promise<bigint | null>;
}

/** A status-list segment cached for offline verification. */
export interface CachedStatusListSegment {
  segment: StatusListSegment;
  /** Epoch the segment was published for; segments are only valid within it. */
  epoch: bigint;
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
  /** Cached segments keyed by epoch, then segment index. */
  private readonly segments = new Map<bigint, Map<number, CachedStatusListSegment>>();
  /** The epoch most recently synced; used as the default for reads. */
  private activeEpoch: bigint | null = null;

  constructor(private readonly reader: ChainReader) {}

  /**
   * Fetch and validate the segment at `segmentIndex` for `epoch`. Returns
   * `null` when the reader cannot serve status lists or the segment is
   * missing. Throws if the segment does not prove inclusion under the signed
   * root for that epoch.
   *
   * The root for `epoch` is read first, then the segment for the same epoch,
   * so a segment can never be validated against a root from another epoch.
   */
  async syncSegment(
    trust: NetworkTrust,
    epoch: bigint,
    segmentIndex: number,
  ): Promise<StatusListSegment | null> {
    if (!this.reader.getStatusListSegment || !this.reader.getStatusListRoot) return null;
    const root = await this.reader.getStatusListRoot(trust, epoch);
    if (!root) return null;
    if (root.epoch !== epoch) {
      throw new Error(
        `status-list root epoch ${root.epoch} does not match requested epoch ${epoch}`,
      );
    }
    const segment = await this.reader.getStatusListSegment(trust, epoch, segmentIndex);
    if (!segment) return null;
    if (segment.epoch !== epoch) {
      throw new Error(
        `status-list segment ${segmentIndex} epoch ${segment.epoch} does not match root epoch ${epoch}`,
      );
    }
    if (!verifySegmentInclusion(root, segment)) {
      throw new Error(`status-list segment ${segmentIndex} failed inclusion check for epoch ${epoch}`);
    }
    let byIndex = this.segments.get(epoch);
    if (!byIndex) {
      byIndex = new Map();
      this.segments.set(epoch, byIndex);
    }
    byIndex.set(segmentIndex, { segment, epoch, fetchedAt: Date.now() / 1000 });
    this.activeEpoch = epoch;
    return segment;
  }

  /**
   * Sync every segment index listed in the signed root for `epoch`. When
   * `epoch` is omitted the newest epoch the reader knows about is used.
   */
  async syncAll(trust: NetworkTrust, epoch?: bigint): Promise<number> {
    if (!this.reader.getStatusListRoot) return 0;
    const target = epoch ?? (await this.resolveEpoch(trust));
    if (target === null) return 0;
    const root = await this.reader.getStatusListRoot(trust, target);
    if (!root) return 0;
    let synced = 0;
    for (const segmentIndex of root.segmentIndexes) {
      if (await this.syncSegment(trust, target, segmentIndex)) synced++;
    }
    return synced;
  }

  /** The newest epoch the reader can serve, or `null` if none. */
  private async resolveEpoch(trust: NetworkTrust): Promise<bigint | null> {
    if (this.reader.getStatusListEpoch) return this.reader.getStatusListEpoch(trust);
    return this.activeEpoch;
  }

  /** The epoch most recently synced, or `null` if nothing has been synced. */
  get epoch(): bigint | null {
    return this.activeEpoch;
  }

  /**
   * A cached segment for `epoch` (default: the active epoch), or `null` if it
   * has not been synced.
   */
  get(segmentIndex: number, epoch?: bigint): StatusListSegment | null {
    const target = epoch ?? this.activeEpoch;
    if (target === null) return null;
    return this.segments.get(target)?.get(segmentIndex)?.segment ?? null;
  }

  /**
   * Whether `recordHash` is revoked according to the cached segments for
   * `epoch` (default: the active epoch).
   */
  isRevoked(recordHash: string, epoch?: bigint): boolean {
    const target = epoch ?? this.activeEpoch;
    if (target === null) return false;
    const byIndex = this.segments.get(target);
    if (!byIndex) return false;
    for (const { segment } of byIndex.values()) {
      if (isRevokedInSegment(segment, recordHash)) return true;
    }
    return false;
  }

  /** Drop all cached segments (e.g. before a fresh sync). */
  clear(): void {
    this.segments.clear();
    this.activeEpoch = null;
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
  /**
   * Epoch the captured status-list segments and root belong to. Snapshots
   * embed it so offline verification reads an epoch-consistent pair.
   */
  statusListEpoch?: string;
  /** Status-list segments captured in the snapshot, keyed by segment index. */
  statusListSegments?: Record<string, StatusListSegment>;
  /** The signed status-list root the captured segments commit to. */
  statusListRoot?: StatusListRoot;
  /** Roots for older epochs still within the retention window, keyed by epoch. */
  statusListEpochRoots?: Record<string, StatusListEpochRoot>;
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

  async getStatusListSegment(_t: NetworkTrust, epoch: bigint, segmentIndex: number) {
    // Only the epoch the snapshot was captured for is served; older epochs
    // are not embedded, so a mismatched read yields nothing rather than a
    // segment from the wrong epoch.
    if (this.bundle.statusListEpoch === undefined) return null;
    if (BigInt(this.bundle.statusListEpoch) !== epoch) return null;
    return this.bundle.statusListSegments?.[String(segmentIndex)] ?? null;
  }

  async getStatusListRoot(_t: NetworkTrust, epoch: bigint) {
    if (this.bundle.statusListEpoch !== undefined && BigInt(this.bundle.statusListEpoch) === epoch) {
      return this.bundle.statusListRoot ?? null;
    }
    return this.bundle.statusListEpochRoots?.[String(epoch)]?.root ?? null;
  }

  async getStatusListEpoch(_t: NetworkTrust) {
    return this.bundle.statusListEpoch === undefined ? null : BigInt(this.bundle.statusListEpoch);
  }
}
