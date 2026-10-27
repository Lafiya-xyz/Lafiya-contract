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
}

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

  constructor(
    private readonly providers: QuorumProvider[],
    options: QuorumOptions = {},
  ) {
    this.k = Math.max(1, options.k ?? 2);
    this.maxLedgerSkew = options.maxLedgerSkew ?? 2;
  }

  getAttestation(trust: NetworkTrust, recordHash: string): Promise<OnChainAttestation | null> {
    return this.quorum(trust, (r) => r.getAttestation(trust, recordHash));
  }

  getAttestationHistory(trust: NetworkTrust, recordHash: string): Promise<OnChainAttestation[]> {
    return this.quorum(trust, (r) => r.getAttestationHistory(trust, recordHash));
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
