import type { Reason, Verdict } from "./verdict.js";

/**
 * Locales with a responder-facing catalog. `pcm` is Nigerian Pidgin
 * (ISO 639-3). English is the deterministic fallback for every other locale.
 */
export const LOCALES = ["en", "ha", "yo", "ig", "pcm"] as const;

export type Locale = (typeof LOCALES)[number];

export const FALLBACK_LOCALE: Locale = "en";

/**
 * How a UI must treat a verdict, independent of any icon or colour: the
 * text carries the meaning on its own, and this level is for screen readers,
 * haptics, or monochrome displays.
 */
export type Severity = "ok" | "caution" | "stop";

export const VERDICT_SEVERITY: Record<Verdict, Severity> = {
  invalid_payload: "stop",
  untrusted_contract: "stop",
  commitment_mismatch: "stop",
  unavailable: "caution",
  revoked: "stop",
  not_found: "stop",
  superseded: "caution",
  attester_removed: "stop",
  attester_suspended: "caution",
  expired: "caution",
  verified: "ok",
};

/** Who wrote and who clinically reviewed one catalog entry. */
export interface Provenance {
  /** `reviewed` only after a native-speaking clinical reviewer signed off. */
  status: "reviewed" | "pending_review";
  author: string;
  /** Native-speaking clinical reviewer; `null` until reviewed. */
  reviewer: string | null;
  /** ISO date of the last change or review. */
  date: string;
}

/** Plain-language text for one verdict (or reason code) in one locale. */
export interface VerdictText {
  /** At most a few words; read first, under stress. */
  title: string;
  /** What this means for the patient's card, in one or two short sentences. */
  guidance: string;
  /** The single next thing to do. */
  action: string;
}

export interface CatalogEntry extends VerdictText {
  provenance: Provenance;
}

/** One locale's strings. Non-English locales may be partial. */
export type LocaleCatalog = Partial<Record<Verdict, CatalogEntry>>;

export interface LocalizedVerdict extends VerdictText {
  verdict: Verdict;
  severity: Severity;
  /** The locale the caller asked for. */
  requested: string;
  /** The locale the text is actually in. */
  locale: Locale;
  /** `true` when the text is the English fallback for another locale. */
  fallback: boolean;
  provenance: Provenance;
}

export interface LocalizeOptions {
  /**
   * Only return strings a clinical reviewer has approved; unreviewed
   * translations fall back to English. Defaults to `true`.
   */
  requireReviewed?: boolean;
}

// Each catalog is a separate module, loaded on first use, so the core
// bundle carries none of them.
function loadCatalog(locale: Locale): Promise<LocaleCatalog> {
  switch (locale) {
    case "en":
      return import("./locales/en.js").then((m) => m.catalog);
    case "ha":
      return import("./locales/ha.js").then((m) => m.catalog);
    case "yo":
      return import("./locales/yo.js").then((m) => m.catalog);
    case "ig":
      return import("./locales/ig.js").then((m) => m.catalog);
    case "pcm":
      return import("./locales/pcm.js").then((m) => m.catalog);
  }
}

const cache = new Map<Locale, Promise<LocaleCatalog>>();

/** Load (once) and return the raw catalog for a supported locale. */
export function getCatalog(locale: Locale): Promise<LocaleCatalog> {
  let catalog = cache.get(locale);
  if (!catalog) {
    catalog = loadCatalog(locale);
    cache.set(locale, catalog);
    // Let a failed load be retried on the next call.
    catalog.catch(() => cache.delete(locale));
  }
  return catalog;
}

/** Resolve a BCP 47 tag (`"ha-NG"`, `"PCM"`) to a supported locale. */
export function resolveLocale(tag: string): Locale | null {
  const base = tag.trim().toLowerCase().split(/[-_]/)[0];
  return (LOCALES as readonly string[]).includes(base) ? (base as Locale) : null;
}

/**
 * Responder-facing text for a verdict in `locale`. Falls back to English,
 * with `fallback: true`, when the locale is unsupported, the entry is
 * missing, or (by default) the entry has not been clinically reviewed.
 */
export async function localizeVerdict(
  verdict: Verdict,
  locale: string,
  options: LocalizeOptions = {},
): Promise<LocalizedVerdict> {
  const requireReviewed = options.requireReviewed ?? true;
  const resolved = resolveLocale(locale);
  if (resolved && resolved !== FALLBACK_LOCALE) {
    const entry = (await getCatalog(resolved))[verdict];
    if (entry && (!requireReviewed || entry.provenance.status === "reviewed")) {
      return build(verdict, locale, resolved, false, entry);
    }
  }
  const english = (await getCatalog(FALLBACK_LOCALE))[verdict]!;
  return build(verdict, locale, FALLBACK_LOCALE, resolved !== FALLBACK_LOCALE, english);
}

/** {@link localizeVerdict} for one reason; reason codes share the verdict keys. */
export function localizeReason(
  reason: Reason,
  locale: string,
  options?: LocalizeOptions,
): Promise<LocalizedVerdict> {
  return localizeVerdict(reason.code, locale, options);
}

function build(
  verdict: Verdict,
  requested: string,
  locale: Locale,
  fallback: boolean,
  entry: CatalogEntry,
): LocalizedVerdict {
  const { title, guidance, action, provenance } = entry;
  return {
    verdict,
    severity: VERDICT_SEVERITY[verdict],
    requested,
    locale,
    fallback,
    title,
    guidance,
    action,
    provenance,
  };
}
