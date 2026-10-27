import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  backoffDelay,
  callWithPolicy,
  classifyError,
  encodeQrPayload,
  isTransportError,
  LafiyaVerifier,
  TransportError,
  type ChainReader,
  type RetryPolicy,
} from "../src/index.js";
import { CHW, NOW, REGISTRY, TRUST } from "./helpers.js";

const RETRY: RetryPolicy = { retries: 2, baseDelayMs: 100, maxDelayMs: 1_000, random: () => 0.5 };

/** A call that never settles unless its signal aborts it. */
function hang(signal: AbortSignal) {
  return new Promise<never>((_, reject) => {
    signal.addEventListener("abort", () => reject(signal.reason));
  });
}

// Any unhandled rejection fails the test that produced it.
const unhandled: unknown[] = [];
const onUnhandled = (reason: unknown) => unhandled.push(reason);

beforeEach(() => {
  vi.useFakeTimers();
  unhandled.length = 0;
  process.on("unhandledRejection", onUnhandled);
});

afterEach(async () => {
  vi.useRealTimers();
  await new Promise((r) => setTimeout(r, 0));
  process.off("unhandledRejection", onUnhandled);
  expect(unhandled).toEqual([]);
});

describe("callWithPolicy", () => {
  it("times out a hung call", async () => {
    const call = callWithPolicy(hang, { timeoutMs: 1_000 });
    const result = expect(call).rejects.toMatchObject({ kind: "timeout", attempts: 1 });
    await vi.advanceTimersByTimeAsync(1_000);
    await result;
  });

  it("does not time out before the deadline", async () => {
    let settled = false;
    const call = callWithPolicy(hang, { timeoutMs: 1_000 }).catch(() => (settled = true));
    await vi.advanceTimersByTimeAsync(999);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    await call;
    expect(settled).toBe(true);
  });

  it("aborts a hung call and forwards the abort to the attempt", async () => {
    const controller = new AbortController();
    let inner: AbortSignal | undefined;
    const call = callWithPolicy((s) => ((inner = s), hang(s)), { signal: controller.signal });
    controller.abort();
    await expect(call).rejects.toMatchObject({ kind: "aborted" });
    expect(inner?.aborted).toBe(true);
  });

  it("rejects immediately when the signal is already aborted", async () => {
    const fn = vi.fn(hang);
    await expect(callWithPolicy(fn, { signal: AbortSignal.abort() })).rejects.toMatchObject({
      kind: "aborted",
    });
    expect(fn).not.toHaveBeenCalled();
  });

  it("retries timeouts of idempotent reads with jittered backoff", async () => {
    const fn = vi.fn(hang);
    const call = callWithPolicy(fn, { timeoutMs: 1_000, retry: RETRY, idempotent: true });
    const result = expect(call).rejects.toMatchObject({ kind: "timeout", attempts: 3 });
    await vi.advanceTimersByTimeAsync(1_000); // attempt 1 times out
    expect(fn).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(50); // backoff 0.5 * 100
    expect(fn).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1_000 + 100); // attempt 2 + backoff 0.5 * 200
    expect(fn).toHaveBeenCalledTimes(3);
    await vi.advanceTimersByTimeAsync(1_000);
    await result;
  });

  it("succeeds on a retry after a network error", async () => {
    const fn = vi
      .fn<(s: AbortSignal) => Promise<string>>()
      .mockRejectedValueOnce(Object.assign(new Error("reset"), { code: "ECONNRESET" }))
      .mockResolvedValueOnce("ok");
    const call = callWithPolicy(fn, { retry: RETRY, idempotent: true });
    await vi.advanceTimersByTimeAsync(50);
    await expect(call).resolves.toBe("ok");
  });

  it("never retries non-idempotent calls", async () => {
    const fn = vi.fn(hang);
    const call = callWithPolicy(fn, { timeoutMs: 1_000, retry: RETRY });
    const result = expect(call).rejects.toMatchObject({ kind: "timeout", attempts: 1 });
    await vi.advanceTimersByTimeAsync(10_000);
    await result;
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("does not retry protocol errors", async () => {
    const fn = vi.fn(async () => {
      throw new Error("HostError: contract trapped");
    });
    await expect(callWithPolicy(fn, { retry: RETRY, idempotent: true })).rejects.toMatchObject({
      kind: "protocol",
    });
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("aborts during backoff without starting another attempt", async () => {
    const controller = new AbortController();
    const fn = vi.fn(hang);
    const call = callWithPolicy(fn, {
      timeoutMs: 1_000,
      retry: RETRY,
      idempotent: true,
      signal: controller.signal,
    });
    const result = expect(call).rejects.toMatchObject({ kind: "aborted" });
    await vi.advanceTimersByTimeAsync(1_000);
    controller.abort();
    await result;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("swallows a late rejection after a timeout", async () => {
    let rejectLate: (e: unknown) => void = () => {};
    const call = callWithPolicy(
      () => new Promise((_, reject) => (rejectLate = reject)),
      { timeoutMs: 10 },
    );
    const result = expect(call).rejects.toMatchObject({ kind: "timeout" });
    await vi.advanceTimersByTimeAsync(10);
    await result;
    rejectLate(new Error("too late")); // must not become an unhandled rejection
  });

  it("caps backoff at maxDelayMs", () => {
    expect(backoffDelay({ ...RETRY, random: () => 0.999 }, 10)).toBe(999);
  });
});

describe("classifyError", () => {
  it.each([
    [Object.assign(new Error("x"), { name: "AbortError" }), "aborted"],
    [Object.assign(new Error("x"), { code: "ECONNREFUSED" }), "network"],
    [new TypeError("fetch failed"), "network"],
    [Object.assign(new Error("x"), { response: { status: 503 } }), "network"],
    [Object.assign(new Error("x"), { response: { status: 429 } }), "network"],
    [Object.assign(new Error("x"), { response: { status: 404 } }), "not_found"],
    [Object.assign(new Error("x"), { response: { status: 400 } }), "protocol"],
    [new Error("Cannot connect to insecure Soroban RPC server if allowHttp isn't set"), "untrusted"],
    [new Error("bad XDR"), "protocol"],
    ["weird", "protocol"],
  ] as const)("%s -> %s", (err, kind) => {
    const e = classifyError(err);
    expect(e).toBeInstanceOf(TransportError);
    expect(isTransportError(e, kind)).toBe(true);
  });

  it("only retries network and timeout", () => {
    expect(new TransportError("network", "").retryable).toBe(true);
    expect(new TransportError("timeout", "").retryable).toBe(true);
    for (const kind of ["aborted", "protocol", "not_found", "untrusted"] as const) {
      expect(new TransportError(kind, "").retryable).toBe(false);
    }
  });
});

describe("LafiyaVerifier with signal/timeout", () => {
  const card = encodeQrPayload({ network: "testnet", contractId: REGISTRY, recordHash: "ab".repeat(32) });
  const hungChain: ChainReader = {
    source: "rpc",
    getAttestation: (_t, _r, opts) => callWithPolicy(hang, opts),
    getAttestationHistory: (_t, _r, opts) => callWithPolicy(hang, opts),
    getAttesterStatus: async () => ({ suspended: false }),
  };
  const verifier = new LafiyaVerifier({ network: "testnet", trust: TRUST, chain: hungChain, now: () => NOW });

  it("returns unavailable with a timeout error instead of hanging", async () => {
    const pending = verifier.verifyCard(card, { timeoutMs: 2_000 });
    await vi.advanceTimersByTimeAsync(2_000);
    const result = await pending;
    expect(result.verdict).toBe("unavailable");
    expect(result.transportError?.kind).toBe("timeout");
  });

  it("returns unavailable with an aborted error when cancelled", async () => {
    const controller = new AbortController();
    const pending = verifier.verifyCard(card, { signal: controller.signal });
    controller.abort();
    const result = await pending;
    expect(result.verdict).toBe("unavailable");
    expect(result.transportError?.kind).toBe("aborted");
    expect(result.reasons[0].message).toContain("aborted");
  });

  it("passes call options through to the chain reader", async () => {
    const seen: unknown[] = [];
    const chain: ChainReader = {
      source: "rpc",
      getAttestation: async (_t, _r, opts) => (seen.push(opts), { attester: CHW, timestamp: 1n }),
      getAttestationHistory: async () => [],
      getAttesterStatus: async (_t, _a, opts) => (seen.push(opts), { suspended: false }),
    };
    const signal = new AbortController().signal;
    await new LafiyaVerifier({ network: "testnet", trust: TRUST, chain, now: () => NOW }).verifyCard(card, {
      signal,
      timeoutMs: 5,
    });
    expect(seen).toEqual([
      { signal, timeoutMs: 5 },
      { signal, timeoutMs: 5 },
    ]);
  });
});
