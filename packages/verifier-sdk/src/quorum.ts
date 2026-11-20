import type { AttesterState, ChainReader, OnChainAttestation } from "./chain.js";
import type { NetworkTrust } from "./trust.js";

/** Thrown when quorum cannot be reached; the verifier maps it to `unavailable`. */
export class QuorumError extends Error {
  constructor(
    readonly kind: "disagreement" | "insufficient",
    message: string,
  ) {
    super(message);
    this.name = "QuorumError";
  }
}

export interface QuorumProvider {
  reader: ChainReader;
  /** Latest ledger this provider reports; enables stale-provider exclusion. */
  latestLedger?: (trust: NetworkTrust) => Promise<number>;
}

export interface QuorumOptions {
  /** Minimum number of agreeing, fresh providers. Default 2. */
  k?: number;
  /** Providers more than this many ledgers behind the newest are excluded. Default 2. */
  maxLedgerSkew?: number;
  /**
   * Reject co-attestations whose attesters share a licence or facility
   * identity (see {@link assertDistinctAttesters}). Default true.
   */
  enforceDistinctAttesters?: boolean;
}

/**
 * Thrown when co-attestations are not from distinct humans, i.e. two or more
 * attesters share a licence hash or facility identity. The verifier maps it to
 * `unavailable`; it is never a "verified" verdict.
 */
export class QuorumDistinctnessError extends QuorumError {
  constructor(
    readonly reason: "licence" | "facility",
    message: string,
  ) {
    super("disagreement", message);
    this.name = "QuorumDistinctnessError";
  }
}

/**
 * Identity data used to check that co-attesters are distinct humans.
 *
 * Distinctness rule (issue #679): a quorum must be made up of attesters that
 * do not share a licence hash or a facility identity. One health worker (or a
 * stolen licence number) enrolled under several addresses must not be able to
 * satisfy a 2-of-N quorum alone. When the data is unavailable for an attester
 * the check is skipped for that attester, so distinctness is enforced only
 * where the chain exposes the identity.
 */
export interface AttesterIdentity {
  /** Canonical, deployment-salted licence hash, if known. */
  licenseHash?: string;
  /** Facility identity the attester is bound to, if known. */
  facilityId?: string;
}

/** Looks up identity data for an attester address; may return null if unknown. */
export type AttesterIdentityLookup = (attester: string) => Promise<AttesterIdentity | null> | AttesterIdentity | null;

/**
 * Throws {@link QuorumDistinctnessError} if any two attesters share a licence
 * hash or facility identity. Attesters without identity data are ignored.
 */
export function assertDistinctAttesters(
  attesters: readonly string[],
  identities: ReadonlyMap<string, AttesterIdentity | null | undefined>,
): void {
  const seenLicences = new Map<string, string>();
  const seenFacilities = new Map<string, string>();
  for (const attester of attesters) {
    const identity = identities.get(attester);
    if (!identity) continue;
    if (identity.licenseHash) {
      const prior = seenLicences.get(identity.licenseHash);
      if (prior !== undefined && prior !== attester) {
        throw new QuorumDistinctnessError(
          "licence",
          `attesters ${prior} and ${attester} share a licence hash`,
        );
      }
      seenLicences.set(identity.licenseHash, attester);
    }
    if (identity.facilityId) {
      const prior = seenFacilities.get(identity.facilityId);
      if (prior !== undefined && prior !== attester) {
        throw new QuorumDistinctnessError(
          "facility",
          `attesters ${prior} and ${attester} share a facility identity`,
        );
      }
      seenFacilities.set(identity.facilityId, attester);
    }
  }
}
=======

export interface QuorumMetrics {
  reads: number;
  disagreements: number;
  staleExcluded: number;
  providerFailures: number;
  insufficient: number;
  /** Wall-clock milliseconds of the most recent quorum read. */
  lastLatencyMs: number;
}

/**
 * A {@link ChainReader} that cross-checks every critical read across
 * independent providers. Any disagreement between fresh providers, or fewer
 * than `k` answers, throws {@link QuorumError} so the verdict is never
 * "verified" on a single provider's word.
 *
 * Trust assumptions: fewer than `k` providers are malicious or colluding and
 * the providers are independently operated. A provider overstating its
 * ledger can only make others look stale (denial of verification).
 */
export class QuorumChainReader implements ChainReader {
  readonly source = "rpc" as const;
  readonly metrics: QuorumMetrics = {
    reads: 0,
    disagreements: 0,
    staleExcluded: 0,
    providerFailures: 0,
    insufficient: 0,
    lastLatencyMs: 0,
  };
  private readonly k: number;
  private readonly maxLedgerSkew: number;
  private readonly enforceDistinctAttesters: boolean;
  private readonly identityLookup?: AttesterIdentityLookup;

  constructor(
    private readonly providers: QuorumProvider[],
    options: QuorumOptions = {},
    identityLookup?: AttesterIdentityLookup,
  ) {
    this.k = Math.max(1, options.k ?? 2);
    this.maxLedgerSkew = options.maxLedgerSkew ?? 2;
    this.enforceDistinctAttesters = options.enforceDistinctAttesters ?? true;
    this.identityLookup = identityLookup;
  }

  getAttestation(trust: NetworkTrust, recordHash: string): Promise<OnChainAttestation | null> {
    return this.quorum(trust, (r) => r.getAttestation(trust, recordHash));
  }

  async getAttestationHistory(trust: NetworkTrust, recordHash: string): Promise<OnChainAttestation[]> {
    const history = await this.quorum(trust, (r) => r.getAttestationHistory(trust, recordHash));
    await this.checkDistinctness(history);
    return history;
  }

  /**
   * Enforces the distinctness rule on a set of co-attestations when identity
   * data is available. Throws {@link QuorumDistinctnessError} on a shared
   * licence hash or facility identity.
   */
  private async checkDistinctness(history: readonly OnChainAttestation[]): Promise<void> {
    if (!this.enforceDistinctAttesters || !this.identityLookup) return;
    const attesters = [...new Set(history.map((a) => a.attester))];
    if (attesters.length < 2) return;
    const identities = new Map<string, AttesterIdentity | null>();
    await Promise.all(
      attesters.map(async (attester) => {
        try {
          identities.set(attester, await this.identityLookup!(attester));
        } catch {
          identities.set(attester, null);
        }
      }),
    );
    assertDistinctAttesters(attesters, identities);
  }

  getAttesterStatus(trust: NetworkTrust, attester: string): Promise<AttesterState | null> {
    return this.quorum(trust, (r) => r.getAttesterStatus(trust, attester));
  }

  private async quorum<T>(trust: NetworkTrust, read: (r: ChainReader) => Promise<T>): Promise<T> {
    const start = Date.now();
    this.metrics.reads++;
    try {
      const settled = await Promise.all(
        this.providers.map(async (p) => {
          try {
            const [value, ledger] = await Promise.all([
              read(p.reader),
              p.latestLedger ? p.latestLedger(trust) : Promise.resolve(undefined),
            ]);
            return { value, ledger };
          } catch {
            this.metrics.providerFailures++;
            return null;
          }
        }),
      );
      let answers = settled.filter((a): a is { value: T; ledger: number | undefined } => a !== null);
      const newest = Math.max(...answers.map((a) => a.ledger ?? -1));
      if (newest >= 0) {
        const fresh = answers.filter((a) => a.ledger !== undefined && newest - a.ledger <= this.maxLedgerSkew);
        this.metrics.staleExcluded += answers.length - fresh.length;
        answers = fresh;
      }
      const keys = new Set(answers.map((a) => canonical(a.value)));
      if (keys.size > 1) {
        this.metrics.disagreements++;
        throw new QuorumError("disagreement", "providers returned conflicting results");
      }
      if (answers.length < this.k) {
        this.metrics.insufficient++;
        throw new QuorumError("insufficient", `only ${answers.length} of ${this.k} required providers answered`);
      }
      return answers[0]!.value;
    } finally {
      this.metrics.lastLatencyMs = Date.now() - start;
    }
  }
}

function canonical(v: unknown): string {
  return JSON.stringify(v, (_k, x) => (typeof x === "bigint" ? `${x}n` : x));
}
