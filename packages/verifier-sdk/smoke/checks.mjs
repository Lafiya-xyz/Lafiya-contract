// Shared assertions run against the *installed tarball* in every runtime
// (Node ESM, Node CJS, bundled, worker). Throws on the first failure.
export async function run(sdk, runtime) {
  const assert = (cond, what) => {
    if (!cond) throw new Error(`[smoke:${runtime}] ${what}`);
  };
  assert(sdk.VERDICTS.includes("verified"), "VERDICTS exported");
  const payload = sdk.encodeQrPayload({
    network: "testnet",
    contractId: "C" + "A".repeat(55),
    recordHash: "ab".repeat(32),
  });
  assert(sdk.parseQrPayload(payload).recordHash === "ab".repeat(32), "QR round trip");
  const err = sdk.classifyError(Object.assign(new Error("x"), { code: "ECONNRESET" }));
  assert(err instanceof sdk.TransportError && err.kind === "network", "typed transport errors");
  const text = await sdk.localizeVerdict("revoked", "pcm", { requireReviewed: false });
  assert(text.locale === "pcm" && !text.fallback && text.title.length > 0, "lazy locale load");
  const fallback = await sdk.localizeVerdict("revoked", "xx");
  assert(fallback.locale === "en" && fallback.fallback, "English fallback");
  const model = sdk.evaluateModel({
    network_matches: true,
    attestation_read: "ok",
    attester_status_read: "ok",
    attestation_found: true,
    revoked: false,
    attester_status: "active",
    attested_in_compromise_window: false,
    age_exceeds_policy: false,
    superseded: false,
    offline: false,
  });
  assert(model.verdict === "Verified", "verification model");
  assert(typeof sdk.BindingsChainReader === "function", "bindings reader loads");
  return `ok (${runtime})`;
}
