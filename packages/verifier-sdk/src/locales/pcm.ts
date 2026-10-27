import type { LocaleCatalog, Provenance } from "../i18n.js";

// Nigerian Pidgin draft. Not for clinical use until a native-speaking
// clinical reviewer sets `status: "reviewed"` (the SDK falls back to
// English until then).
const p: Provenance = {
  status: "pending_review",
  author: "Lafiya maintainers (draft)",
  reviewer: null,
  date: "2026-10-17",
};

export const catalog: LocaleCatalog = {
  invalid_payload: {
    title: "No be Lafiya card",
    guidance: "Dis code no be Lafiya health card. No use am for treatment.",
    action: "Scan di card again, or ask for another ID.",
    provenance: p,
  },
  untrusted_contract: {
    title: "We no trust dis card",
    guidance: "Dis card come from place wey we no trust. No use am for treatment.",
    action: "Treat di patient like say e no get record.",
    provenance: p,
  },
  commitment_mismatch: {
    title: "Record no match",
    guidance: "Di record wey dem show no be di one wey dey for di card. Dem fit don change am.",
    action: "No use dis record. Ask di patient by yourself.",
    provenance: p,
  },
  unavailable: {
    title: "We no fit check am",
    guidance: "We no fit check di card now. E no mean say e good or e bad.",
    action: "Try again. If e still no work, ask di patient by yourself.",
    provenance: p,
  },
  revoked: {
    title: "Dem don cancel di card",
    guidance: "Dem don cancel dis record. No use am for treatment.",
    action: "Ask di patient by yourself.",
    provenance: p,
  },
  not_found: {
    title: "We no see record",
    guidance: "No health worker sign dis record.",
    action: "No use dis record. Ask di patient by yourself.",
    provenance: p,
  },
  superseded: {
    title: "Old card",
    guidance: "New record dey for dis patient. Dis card fit don old.",
    action: "Use am with care. Ask for di newest card.",
    provenance: p,
  },
  attester_removed: {
    title: "We no trust di person wey sign",
    guidance: "We no trust di health worker wey sign dis record again.",
    action: "No use dis record. Ask di patient by yourself.",
    provenance: p,
  },
  attester_suspended: {
    title: "Di person wey sign dey on hold",
    guidance: "Di health worker wey sign dis record dey on hold for now.",
    action: "Use am with care. Confirm di main tins with di patient.",
    provenance: p,
  },
  expired: {
    title: "Record don old",
    guidance: "Dis record don pass di time wey we allow. E fit don old.",
    action: "Use am with care. Confirm di main tins with di patient.",
    provenance: p,
  },
  verified: {
    title: "Card don check",
    guidance: "Health worker wey we trust sign dis record.",
    action: "You fit use dis record.",
    provenance: p,
  },
};
