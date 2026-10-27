// Lints the responder-facing verdict catalogs (run after `pnpm build`):
//
// - every verdict has an entry in every locale, or an explicit waiver in
//   i18n/waivers.json ({ "<locale>": { "<verdict>": "<why>" } });
// - every waiver still refers to a missing entry (no stale waivers);
// - every entry has non-empty text and complete provenance, and a
//   `reviewed` entry names its reviewer;
// - English source strings stay within the reading-level budget.
import { readFileSync } from "node:fs";

const dist = new URL("../dist/esm/", import.meta.url);
const { VERDICTS } = await import(new URL("verdict.js", dist).href);
const { LOCALES, FALLBACK_LOCALE } = await import(new URL("i18n.js", dist).href);
const waivers = JSON.parse(readFileSync(new URL("../i18n/waivers.json", import.meta.url), "utf8"));

// Reading-level proxies for the English source (target CEFR A2 / grade 5).
const MAX_TITLE_WORDS = 5;
const MAX_SENTENCE_WORDS = 15;
const MAX_WORD_LETTERS = 12;

const errors = [];
const words = (s) => s.trim().split(/\s+/).filter(Boolean);

for (const locale of LOCALES) {
  const { catalog } = await import(new URL(`locales/${locale}.js`, dist).href);
  const waived = waivers[locale] ?? {};
  for (const key of Object.keys(waived)) {
    if (!VERDICTS.includes(key)) errors.push(`${locale}: waiver for unknown verdict "${key}"`);
    else if (catalog[key]) errors.push(`${locale}.${key}: stale waiver (entry exists)`);
  }
  for (const key of Object.keys(catalog)) {
    if (!VERDICTS.includes(key)) errors.push(`${locale}: entry for unknown verdict "${key}"`);
  }
  for (const verdict of VERDICTS) {
    const entry = catalog[verdict];
    if (!entry) {
      if (locale === FALLBACK_LOCALE) errors.push(`${locale}.${verdict}: missing (no waiver possible)`);
      else if (!waived[verdict]) errors.push(`${locale}.${verdict}: missing and not waived`);
      continue;
    }
    for (const field of ["title", "guidance", "action"]) {
      const text = entry[field];
      if (typeof text !== "string" || !text.trim()) errors.push(`${locale}.${verdict}.${field}: empty`);
      else if (/\b[a-z]+_[a-z_]+\b|[A-Z]{2,}_[A-Z_]+/.test(text)) {
        errors.push(`${locale}.${verdict}.${field}: contains a code, not plain language`);
      }
    }
    const p = entry.provenance ?? {};
    if (!["reviewed", "pending_review"].includes(p.status)) errors.push(`${locale}.${verdict}: bad provenance.status`);
    if (!p.author) errors.push(`${locale}.${verdict}: provenance.author missing`);
    if (!/^\d{4}-\d{2}-\d{2}$/.test(p.date ?? "")) errors.push(`${locale}.${verdict}: provenance.date not ISO`);
    if (p.status === "reviewed" && !p.reviewer) errors.push(`${locale}.${verdict}: reviewed without a reviewer`);
    if (locale === FALLBACK_LOCALE) {
      if (words(entry.title).length > MAX_TITLE_WORDS) errors.push(`en.${verdict}.title: over ${MAX_TITLE_WORDS} words`);
      for (const field of ["guidance", "action"]) {
        for (const sentence of entry[field].split(/[.!?]/).filter((s) => s.trim())) {
          if (words(sentence).length > MAX_SENTENCE_WORDS) {
            errors.push(`en.${verdict}.${field}: sentence over ${MAX_SENTENCE_WORDS} words`);
          }
          for (const w of words(sentence)) {
            if (w.replace(/[^A-Za-z]/g, "").length > MAX_WORD_LETTERS) {
              errors.push(`en.${verdict}.${field}: long word "${w}"`);
            }
          }
        }
      }
    }
  }
}

const total = LOCALES.length * VERDICTS.length;
if (errors.length) {
  for (const e of errors) console.error(`[i18n] ${e}`);
  console.error(`[i18n] ${errors.length} problem(s)`);
  process.exit(1);
}
console.log(`[i18n] ${LOCALES.length} locales x ${VERDICTS.length} verdicts ok (${total} entries or waivers)`);
