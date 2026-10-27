import { classifyError, TransportError } from "./errors.js";

/** Per-call cancellation and deadline, accepted by every chain read. */
export interface CallOptions {
  /** Cancels the call (and any pending retry) when it fires. */
  signal?: AbortSignal;
  /** Deadline for each attempt, in milliseconds. */
  timeoutMs?: number;
}

/**
 * Bounded retries with "full jitter" exponential backoff: before attempt
 * `n + 1` the caller waits `random() * min(maxDelayMs, baseDelayMs * 2^n)`.
 */
export interface RetryPolicy {
  /** Extra attempts after the first (0 disables retries). */
  retries: number;
  baseDelayMs: number;
  maxDelayMs: number;
  /** Source of jitter in [0, 1); override for deterministic tests. */
  random?: () => number;
}

export const DEFAULT_TIMEOUT_MS = 10_000;

export const DEFAULT_RETRY: RetryPolicy = { retries: 2, baseDelayMs: 200, maxDelayMs: 2_000 };

export interface PolicyOptions extends CallOptions {
  retry?: RetryPolicy;
  /**
   * Only idempotent reads are retried. Writes are ambiguous (the first
   * attempt may have landed) and must leave this `false`, the default.
   */
  idempotent?: boolean;
}

function abortedError(signal: AbortSignal): TransportError<"aborted"> {
  return new TransportError("aborted", "the call was aborted", { cause: signal.reason });
}

/** Run one attempt under the caller's signal and the per-attempt deadline. */
function attempt<T>(
  fn: (signal: AbortSignal) => Promise<T>,
  outer: AbortSignal | undefined,
  timeoutMs: number | undefined,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    if (outer?.aborted) return reject(abortedError(outer));
    const inner = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let settled = false;
    const finish = (settle: () => void) => {
      if (settled) return;
      settled = true;
      if (timer !== undefined) clearTimeout(timer);
      outer?.removeEventListener("abort", onAbort);
      settle();
    };
    const onAbort = () => {
      inner.abort(outer?.reason);
      finish(() => reject(abortedError(outer!)));
    };
    outer?.addEventListener("abort", onAbort, { once: true });
    if (timeoutMs !== undefined && Number.isFinite(timeoutMs)) {
      timer = setTimeout(() => {
        const err = new TransportError("timeout", `no response within ${timeoutMs} ms`);
        inner.abort(err);
        finish(() => reject(err));
      }, timeoutMs);
    }
    let pending: Promise<T>;
    try {
      pending = fn(inner.signal);
    } catch (err) {
      finish(() => reject(classifyError(err)));
      return;
    }
    // A late rejection after a timeout/abort is swallowed here, so it can
    // never surface as an unhandled rejection.
    pending.then(
      (value) => finish(() => resolve(value)),
      (err: unknown) => finish(() => reject(classifyError(err))),
    );
  });
}

/** Wait `ms`, rejecting early with `aborted` if the signal fires. */
function sleep(ms: number, signal: AbortSignal | undefined): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    if (signal?.aborted) return reject(abortedError(signal));
    const onAbort = () => {
      clearTimeout(timer);
      reject(abortedError(signal!));
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

/** Backoff before retry number `n` (0-based), with full jitter. */
export function backoffDelay(policy: RetryPolicy, n: number): number {
  const cap = Math.min(policy.maxDelayMs, policy.baseDelayMs * 2 ** n);
  return Math.floor((policy.random ?? Math.random)() * cap);
}

/**
 * Run `fn` with cancellation, a per-attempt timeout, and (for idempotent
 * calls only) bounded retries on `network`/`timeout` failures. Always
 * rejects with a {@link TransportError}.
 */
export async function callWithPolicy<T>(
  fn: (signal: AbortSignal) => Promise<T>,
  options: PolicyOptions = {},
): Promise<T> {
  const retries = options.idempotent ? Math.max(0, options.retry?.retries ?? 0) : 0;
  for (let n = 0; ; n++) {
    try {
      return await attempt(fn, options.signal, options.timeoutMs);
    } catch (err) {
      const error = classifyError(err);
      if (n >= retries || !error.retryable) {
        throw new TransportError(error.kind, error.message, {
          cause: error.cause ?? err,
          attempts: n + 1,
        });
      }
      await sleep(backoffDelay(options.retry!, n), options.signal);
    }
  }
}
