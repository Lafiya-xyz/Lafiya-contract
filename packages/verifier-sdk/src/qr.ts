/**
 * Lafiya card QR payload, version 1:
 *
 *   lafiya:v1?network=<name>&contract=<C... attestation registry>&record=<64 hex>[&attested=<unix seconds>]
 *
 * `network` and `contract` are *claims*: the verifier only proceeds if they
 * match its trust anchors. `attested` is the attestation timestamp the card
 * was issued with, used to detect a superseded card.
 *
 * Freshness extension (backwards compatible within v1): optional
 * `issued_at` and `valid_until` unix-second fields. Both are canonical
 * decimal integers (no sign, no leading zeros, at most 20 digits) and
 * `valid_until` must be greater than or equal to `issued_at`. Payloads
 * without these fields remain valid v1 payloads; verifiers apply their
 * configured staleness policy to decide whether to accept them.
 */
export interface QrPayload {
  network: string;
  contractId: string;
  recordHash: string;
  attestedAt?: bigint;
  issuedAt?: bigint;
  validUntil?: bigint;
}

/** How a verifier reacts to a stale, future-dated or missing freshness window. */
export type StalenessPolicy = "warn" | "soft-fail" | "hard-fail";

export type FreshnessStatus =
  | "fresh"
  | "missing"
  | "expired"
  | "future-dated"
  | "invalid-window";

export interface FreshnessVerdict {
  status: FreshnessStatus;
  /** True when the payload may be presented as current. */
  acceptable: boolean;
  /** True when the verifier must refuse to proceed. */
  fatal: boolean;
  /** Human-readable reason, suitable for surfacing to the operator. */
  reason: string;
}

export interface FreshnessOptions {
  /** Policy applied to non-fresh payloads. Defaults to `warn`. */
  policy?: StalenessPolicy;
  /** Verifier clock in unix seconds. Defaults to `Date.now() / 1000`. */
  now?: bigint;
  /**
   * Tolerated clock skew in seconds for offline verifiers with unreliable
   * clocks. A payload is not considered future-dated until `issued_at`
   * exceeds `now + clockSkewSeconds`. Defaults to 300 (5 minutes).
   */
  clockSkewSeconds?: bigint;
}

export class QrPayloadError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "QrPayloadError";
  }
}

const PREFIX = "lafiya:v1?";

export function parseQrPayload(text: string): QrPayload {
  const trimmed = text.trim();
  if (!trimmed.startsWith(PREFIX)) {
    throw new QrPayloadError("not a lafiya:v1 payload");
  }
  const params = new URLSearchParams(trimmed.slice(PREFIX.length));
  const network = params.get("network") ?? "";
  const contractId = params.get("contract") ?? "";
  const recordHash = (params.get("record") ?? "").toLowerCase();
  const attested = params.get("attested");
  const issuedAt = params.get("issued_at");
  const validUntil = params.get("valid_until");

  if (!/^[a-z][a-z0-9_-]{0,31}$/.test(network)) {
    throw new QrPayloadError("missing or invalid network");
  }
  if (!/^C[A-Z2-7]{55}$/.test(contractId)) {
    throw new QrPayloadError("missing or invalid contract id");
  }
  if (!/^[0-9a-f]{64}$/.test(recordHash)) {
    throw new QrPayloadError("record must be a 32-byte hex hash");
  }
  if (attested !== null && !/^\d{1,20}$/.test(attested)) {
    throw new QrPayloadError("attested must be unix seconds");
  }
  if (issuedAt !== null && !/^\d{1,20}$/.test(issuedAt)) {
    throw new QrPayloadError("issued_at must be unix seconds");
  }
  if (validUntil !== null && !/^\d{1,20}$/.test(validUntil)) {
    throw new QrPayloadError("valid_until must be unix seconds");
  }
  if (issuedAt !== null && validUntil !== null && BigInt(validUntil) < BigInt(issuedAt)) {
    throw new QrPayloadError("valid_until must not precede issued_at");
  }
  return {
    network,
    contractId,
    recordHash,
    ...(attested !== null ? { attestedAt: BigInt(attested) } : {}),
    ...(issuedAt !== null ? { issuedAt: BigInt(issuedAt) } : {}),
    ...(validUntil !== null ? { validUntil: BigInt(validUntil) } : {}),
  };
}

export function encodeQrPayload(payload: QrPayload): string {
  const params = new URLSearchParams({
    network: payload.network,
    contract: payload.contractId,
    record: payload.recordHash,
  });
  if (payload.attestedAt !== undefined) {
    params.set("attested", payload.attestedAt.toString());
  }
  if (payload.issuedAt !== undefined) {
    params.set("issued_at", payload.issuedAt.toString());
  }
  if (payload.validUntil !== undefined) {
    params.set("valid_until", payload.validUntil.toString());
  }
  return PREFIX + params.toString();
}

/**
 * Evaluate the freshness window of a parsed payload against the verifier's
 * policy. Offline verifiers with unreliable clocks should pass a generous
 * `clockSkewSeconds`; a payload is only treated as future-dated once it is
 * beyond that skew.
 */
export function evaluateFreshness(
  payload: QrPayload,
  options: FreshnessOptions = {},
): FreshnessVerdict {
  const policy = options.policy ?? "warn";
  const now = options.now ?? BigInt(Math.floor(Date.now() / 1000));
  const skew = options.clockSkewSeconds ?? 300n;

  const verdict = (status: FreshnessStatus, reason: string): FreshnessVerdict => {
    if (status === "fresh") {
      return { status, acceptable: true, fatal: false, reason };
    }
    return {
      status,
      acceptable: policy === "warn",
      fatal: policy === "hard-fail",
      reason,
    };
  };

  if (
    payload.issuedAt !== undefined &&
    payload.validUntil !== undefined &&
    payload.validUntil < payload.issuedAt
  ) {
    return verdict("invalid-window", "valid_until precedes issued_at");
  }
  if (payload.issuedAt === undefined && payload.validUntil === undefined) {
    return verdict("missing", "payload carries no freshness window");
  }
  if (payload.issuedAt !== undefined && payload.issuedAt > now + skew) {
    return verdict("future-dated", "issued_at is in the future");
  }
  if (payload.validUntil !== undefined && payload.validUntil < now) {
    return verdict("expired", "valid_until has passed");
  }
  return verdict("fresh", "payload is within its validity window");
}

/**
 * Shared parity vectors exercised by both the TypeScript codec and the Rust
 * `lafiya-qr` WASM build. Keep in sync with `crates/lafiya-qr` test vectors.
 */
export interface QrVector {
  name: string;
  text: string;
  /** Expected parse result, or `"error"` when parsing must fail. */
  expected: QrPayload | "error";
  /** Expected freshness status when `expected` is a payload. */
  freshness?: FreshnessStatus;
}

export const QR_PARITY_VECTORS: readonly QrVector[] = [
  {
    name: "missing freshness fields",
    text: "lafiya:v1?network=testnet&contract=CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF&record=" +
      "0000000000000000000000000000000000000000000000000000000000000000",
    expected: {
      network: "testnet",
      contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
      recordHash: "0000000000000000000000000000000000000000000000000000000000000000",
    },
    freshness: "missing",
  },
  {
    name: "valid window",
    text: "lafiya:v1?network=testnet&contract=CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF&record=" +
      "0000000000000000000000000000000000000000000000000000000000000000&issued_at=1700000000&valid_until=1700086400",
    expected: {
      network: "testnet",
      contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
      recordHash: "0000000000000000000000000000000000000000000000000000000000000000",
      issuedAt: 1700000000n,
      validUntil: 1700086400n,
    },
    freshness: "fresh",
  },
  {
    name: "expired window",
    text: "lafiya:v1?network=testnet&contract=CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF&record=" +
      "0000000000000000000000000000000000000000000000000000000000000000&issued_at=1600000000&valid_until=1600086400",
    expected: {
      network: "testnet",
      contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
      recordHash: "0000000000000000000000000000000000000000000000000000000000000000",
      issuedAt: 1600000000n,
      validUntil: 1600086400n,
    },
    freshness: "expired",
  },
  {
    name: "future-dated window",
    text: "lafiya:v1?network=testnet&contract=CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF&record=" +
      "0000000000000000000000000000000000000000000000000000000000000000&issued_at=4102444800&valid_until=4102531200",
    expected: {
      network: "testnet",
      contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
      recordHash: "0000000000000000000000000000000000000000000000000000000000000000",
      issuedAt: 4102444800n,
      validUntil: 4102531200n,
    },
    freshness: "future-dated",
  },
  {
    name: "non-canonical issued_at",
    text: "lafiya:v1?network=testnet&contract=CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF&record=" +
      "0000000000000000000000000000000000000000000000000000000000000000&issued_at=01700000000",
    expected: "error",
  },
  {
    name: "inverted window",
    text: "lafiya:v1?network=testnet&contract=CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF&record=" +
      "0000000000000000000000000000000000000000000000000000000000000000&issued_at=1700086400&valid_until=1700000000",
    expected: "error",
  },
];
