/**
 * Every way a chain read can fail. The set is closed so a UI can switch over
 * `error.kind` exhaustively:
 *
 * - `network`: the request did not get a usable answer (connection refused,
 *   DNS, reset, HTTP 5xx/429). Retryable for idempotent reads.
 * - `timeout`: no answer within the per-call deadline. Retryable for
 *   idempotent reads.
 * - `aborted`: the caller's `AbortSignal` fired. Never retried.
 * - `protocol`: the server answered, but not with something we can use
 *   (malformed response, simulation/contract error, HTTP 4xx). Not retried.
 * - `not_found`: the RPC endpoint or contract does not exist (HTTP 404).
 *   Not retried. A missing *attestation* is not an error: it is `null`.
 * - `untrusted`: the request was refused before it was sent because the
 *   transport cannot be trusted (e.g. plain HTTP without `allowHttp`).
 * - `archived`: the server answered, but the persistent entry (attestation,
 *   attester info, work item) has lapsed its TTL and is archived. The entry
 *   still exists and can be restored; this is a distinct state, not a
 *   verification failure. Not retried as-is: a restore footprint must be
 *   submitted first (see `isArchivedError`).
 */
export const TRANSPORT_ERROR_KINDS = [
  "network",
  "timeout",
  "aborted",
  "protocol",
  "not_found",
  "untrusted",
  "archived",
] as const;

export type TransportErrorKind = (typeof TRANSPORT_ERROR_KINDS)[number];

/** A classified chain-read failure. Narrow on `kind`. */
export class TransportError<K extends TransportErrorKind = TransportErrorKind> extends Error {
  override readonly name = "TransportError";
  /** Number of attempts made before giving up (1 when not retried). */
  readonly attempts: number;

  constructor(
    readonly kind: K,
    message: string,
    options: { cause?: unknown; attempts?: number } = {},
  ) {
    super(message, options.cause === undefined ? undefined : { cause: options.cause });
    this.attempts = options.attempts ?? 1;
  }

  /** Whether an idempotent read may be retried after this error. */
  get retryable(): boolean {
    return this.kind === "network" || this.kind === "timeout";
  }
}

export type NetworkError = TransportError<"network">;
export type TimeoutError = TransportError<"timeout">;
export type AbortedError = TransportError<"aborted">;
export type ProtocolError = TransportError<"protocol">;
export type NotFoundError = TransportError<"not_found">;
export type UntrustedError = TransportError<"untrusted">;
export type ArchivedError = TransportError<"archived">;

/** Type guard, optionally narrowed to one kind. */
export function isTransportError<K extends TransportErrorKind>(
  err: unknown,
  kind?: K,
): err is TransportError<K> {
  return err instanceof TransportError && (kind === undefined || err.kind === kind);
}

/** Whether an error means the entry is archived (restorable) rather than missing. */
export function isArchivedError(err: unknown): err is ArchivedError {
  return isTransportError(err, "archived");
}

/**
 * Build the `archived` error for a persistent entry. `keys` are the ledger
 * keys (base64 XDR) whose TTL lapsed, when the caller knows them.
 */
export function archivedError(
  message: string,
  options: { cause?: unknown; keys?: readonly string[] } = {},
): ArchivedError {
  const err = new TransportError("archived", message, { cause: options.cause });
  if (options.keys !== undefined) {
    Object.defineProperty(err, "keys", {
      value: Object.freeze([...options.keys]),
      enumerable: true,
    });
  }
  return err;
}

const ARCHIVED_PATTERN = /archived|restorePreamble|restore_footprint|entry is archived/i;

/** Whether a thrown value or message describes an archived (restorable) entry. */
export function looksArchived(err: unknown): boolean {
  if (isArchivedError(err)) return true;
  const message = err instanceof Error ? err.message : String(err);
  return ARCHIVED_PATTERN.test(message);
}

const NETWORK_CODES = new Set([
  "ECONNREFUSED",
  "ECONNRESET",
  "ENOTFOUND",
  "EAI_AGAIN",
  "ETIMEDOUT",
  "EPIPE",
  "ENETUNREACH",
  "EHOSTUNREACH",
  "ECONNABORTED",
  "ERR_NETWORK",
  "UND_ERR_SOCKET",
  "UND_ERR_CONNECT_TIMEOUT",
]);

function httpStatus(err: Record<string, unknown>): number | undefined {
  const response = err.response as Record<string, unknown> | undefined;
  const status = response?.status ?? err.status;
  return typeof status === "number" ? status : undefined;
}

/** Map an arbitrary thrown value (axios, fetch, bindings) onto the taxonomy. */
export function classifyError(err: unknown): TransportError {
  if (err instanceof TransportError) return err;
  const message = err instanceof Error ? err.message : String(err);
  if (typeof err === "object" && err !== null) {
    const e = err as Record<string, unknown>;
    if (e.name === "AbortError" || e.code === "ERR_CANCELED") {
      return new TransportError("aborted", message, { cause: err });
    }
    if (e.name === "TimeoutError") return new TransportError("timeout", message, { cause: err });
    const status = httpStatus(e);
    if (status !== undefined) {
      if (status === 404) return new TransportError("not_found", message, { cause: err });
      if (status === 429 || status >= 500) {
        return new TransportError("network", message, { cause: err });
      }
      if (looksArchived(err)) return archivedError(message, { cause: err });
      return new TransportError("protocol", message, { cause: err });
    }
    if (looksArchived(err)) return archivedError(message, { cause: err });
    const code = typeof e.code === "string" ? e.code : undefined;
    const causeCode = (e.cause as Record<string, unknown> | undefined)?.code;
    if (
      (code && NETWORK_CODES.has(code)) ||
      (typeof causeCode === "string" && NETWORK_CODES.has(causeCode)) ||
      (err instanceof TypeError && /fetch|network/i.test(message))
    ) {
      return new TransportError("network", message, { cause: err });
    }
  }
  if (/insecure|allowHttp/i.test(message)) {
    return new TransportError("untrusted", message, { cause: err });
  }
  if (looksArchived(err)) return archivedError(message, { cause: err });
  return new TransportError("protocol", message, { cause: err });
}
