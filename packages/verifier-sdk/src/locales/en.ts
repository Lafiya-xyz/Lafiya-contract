import type { CatalogEntry, Provenance } from "../i18n.js";
import type { Verdict } from "../verdict.js";

// Source strings. Target: CEFR A2 / US grade 5 reading level, short
// sentences, one action, no jargon or reason codes (see docs/verifier-sdk.md).
const p: Provenance = {
  status: "pending_review",
  author: "Lafiya maintainers",
  reviewer: null,
  date: "2026-10-17",
};

export const catalog: Record<Verdict, CatalogEntry> = {
  invalid_payload: {
    title: "Not a Lafiya card",
    guidance: "This code is not a Lafiya health card. Do not use it for care.",
    action: "Scan the card again, or ask for another ID.",
    provenance: p,
  },
  untrusted_contract: {
    title: "Card not trusted",
    guidance: "This card comes from a source we do not trust. Do not use it for care.",
    action: "Treat the patient as having no record.",
    provenance: p,
  },
  commitment_mismatch: {
    title: "Record does not match",
    guidance: "The record shown is not the record on the card. It may be changed.",
    action: "Do not use this record. Ask the patient directly.",
    provenance: p,
  },
  unavailable: {
    title: "Could not check",
    guidance: "We could not check the card right now. It is not good or bad.",
    action: "Try again. If it still fails, ask the patient directly.",
    provenance: p,
  },
  revoked: {
    title: "Card cancelled",
    guidance: "This record was cancelled. Do not use it for care.",
    action: "Ask the patient directly.",
    provenance: p,
  },
  not_found: {
    title: "No record found",
    guidance: "No health worker has signed this record.",
    action: "Do not use this record. Ask the patient directly.",
    provenance: p,
  },
  superseded: {
    title: "Old card",
    guidance: "There is a newer record for this patient. This card may be out of date.",
    action: "Use it with care. Ask for the newest card.",
    provenance: p,
  },
  attester_removed: {
    title: "Signer not trusted",
    guidance: "The health worker who signed this record is no longer trusted.",
    action: "Do not use this record. Ask the patient directly.",
    provenance: p,
  },
  attester_suspended: {
    title: "Signer on hold",
    guidance: "The health worker who signed this record is on hold for now.",
    action: "Use it with care. Check key facts with the patient.",
    provenance: p,
  },
  expired: {
    title: "Record too old",
    guidance: "This record is older than allowed. It may be out of date.",
    action: "Use it with care. Check key facts with the patient.",
    provenance: p,
  },
  verified: {
    title: "Card checked",
    guidance: "A trusted health worker signed this record.",
    action: "You can use this record.",
    provenance: p,
  },
};
