import type { Verdict } from "../../src/index.js";
import type { VerdictStyle } from "./a11y-checklist.js";

const caution = { icon: "⚠", bg: "#5c3b00", fg: "#ffffff", positive: false };
const stop = { icon: "✖", bg: "#7a0b0b", fg: "#ffffff", positive: false };

export const VERDICT_STYLES: Record<Verdict, VerdictStyle> = {
  verified: { icon: "✔", label: "VERIFIED", bg: "#0b3d1a", fg: "#ffffff", positive: true },
  unavailable: { icon: "?", label: "UNAVAILABLE", bg: "#2b2b2b", fg: "#ffffff", positive: false },
  not_found: { icon: "✖", label: "NOT VERIFIED", bg: "#2b2b2b", fg: "#ffffff", positive: false },
  revoked: { ...stop, label: "REVOKED" },
  superseded: { ...caution, label: "OUT OF DATE" },
  expired: { ...caution, label: "EXPIRED" },
  attester_suspended: { ...caution, label: "CAUTION" },
  attester_removed: { ...caution, label: "CAUTION" },
  commitment_mismatch: { ...stop, label: "DOES NOT MATCH" },
  invalid_payload: { ...stop, label: "NOT A LAFIYA CARD" },
  untrusted_contract: { ...stop, label: "UNTRUSTED CARD" },
};
