import { describe, expect, it } from "vitest";
import {
  getCatalog,
  localizeReason,
  localizeVerdict,
  LOCALES,
  resolveLocale,
  VERDICT_SEVERITY,
  VERDICTS,
} from "../src/index.js";
import { catalog as en } from "../src/locales/en.js";

describe("localizeVerdict", () => {
  it("returns English without a fallback flag for en", async () => {
    const text = await localizeVerdict("revoked", "en-NG");
    expect(text).toMatchObject({ locale: "en", requested: "en-NG", fallback: false, severity: "stop" });
    expect(text.title).toBe(en.revoked.title);
  });

  it("falls back to English, flagged, for an unsupported locale", async () => {
    const text = await localizeVerdict("verified", "fr");
    expect(text).toMatchObject({ locale: "en", requested: "fr", fallback: true, title: en.verified.title });
  });

  it("falls back to English for unreviewed translations by default", async () => {
    const catalog = await getCatalog("ha");
    expect(catalog.expired?.provenance.status).toBe("pending_review");
    const text = await localizeVerdict("expired", "ha");
    expect(text).toMatchObject({ locale: "en", fallback: true, title: en.expired.title });
  });

  it("returns the draft when unreviewed strings are explicitly allowed", async () => {
    for (const locale of ["ha", "yo", "ig", "pcm"] as const) {
      const catalog = await getCatalog(locale);
      const text = await localizeVerdict("not_found", locale, { requireReviewed: false });
      expect(text).toMatchObject({ locale, fallback: false, title: catalog.not_found?.title });
    }
  });

  it("is deterministic", async () => {
    const a = await localizeVerdict("superseded", "yo");
    const b = await localizeVerdict("superseded", "yo");
    expect(a).toEqual(b);
  });

  it("localizes reasons through the same keys", async () => {
    const text = await localizeReason({ code: "attester_suspended", message: "x" }, "pcm", {
      requireReviewed: false,
    });
    expect(text.verdict).toBe("attester_suspended");
    expect(text.locale).toBe("pcm");
  });

  it("resolves BCP 47 tags", () => {
    expect(resolveLocale("ha-NG")).toBe("ha");
    expect(resolveLocale("PCM")).toBe("pcm");
    expect(resolveLocale("yo_NG")).toBe("yo");
    expect(resolveLocale("de")).toBeNull();
  });

  it("assigns every verdict a severity, with only verified as ok", () => {
    for (const v of VERDICTS) expect(VERDICT_SEVERITY[v] === "ok").toBe(v === "verified");
  });

  it("loads every catalog", async () => {
    for (const locale of LOCALES) expect(Object.keys(await getCatalog(locale)).length).toBeGreaterThan(0);
  });
});
