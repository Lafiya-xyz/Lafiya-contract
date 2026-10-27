import type { LocaleCatalog, Provenance } from "../i18n.js";

// Igbo draft. Not for clinical use until a native-speaking clinical
// reviewer sets `status: "reviewed"` (the SDK falls back to English until
// then).
const p: Provenance = {
  status: "pending_review",
  author: "Lafiya maintainers (draft)",
  reviewer: null,
  date: "2026-10-17",
};

export const catalog: LocaleCatalog = {
  invalid_payload: {
    title: "Ọ bụghị kaadị Lafiya",
    guidance: "Koodu a abụghị kaadị ahụike Lafiya. Ejila ya agwọ ọrịa.",
    action: "Nyochaa kaadị ahụ ọzọ, ma ọ bụ rịọ maka njirimara ọzọ.",
    provenance: p,
  },
  untrusted_contract: {
    title: "Anyị atụkwasịghị kaadị a obi",
    guidance: "Kaadị a si n'ebe anyị na-atụkwasịghị obi. Ejila ya agwọ ọrịa.",
    action: "Were onye ọrịa ahụ ka onye na-enweghị ndekọ.",
    provenance: p,
  },
  commitment_mismatch: {
    title: "Ndekọ adabaghị",
    guidance: "Ndekọ e gosiri abụghị nke dị na kaadị ahụ. E nwere ike ịgbanwe ya.",
    action: "Ejila ndekọ a. Jụọ onye ọrịa ahụ ozugbo.",
    provenance: p,
  },
  unavailable: {
    title: "Anyị enweghị ike ịlele ya",
    guidance: "Anyị enweghị ike ịlele kaadị ahụ ugbu a. Ọ pụtaghị na ọ dị mma ma ọ bụ na ọ dịghị mma.",
    action: "Nwaa ọzọ. Ọ bụrụ na ọ ka na-arụghị, jụọ onye ọrịa ahụ ozugbo.",
    provenance: p,
  },
  revoked: {
    title: "E kagburu kaadị ahụ",
    guidance: "E kagburu ndekọ a. Ejila ya agwọ ọrịa.",
    action: "Jụọ onye ọrịa ahụ ozugbo.",
    provenance: p,
  },
  not_found: {
    title: "Ahụghị ndekọ ọ bụla",
    guidance: "Ọ dịghị onye ọrụ ahụike bịanyere aka na ndekọ a.",
    action: "Ejila ndekọ a. Jụọ onye ọrịa ahụ ozugbo.",
    provenance: p,
  },
  superseded: {
    title: "Kaadị ochie",
    guidance: "Enwere ndekọ ọhụrụ maka onye ọrịa a. Kaadị a nwere ike ịbụ nke ochie.",
    action: "Jiri ya kpachara anya. Rịọ maka kaadị kachasị ọhụrụ.",
    provenance: p,
  },
  attester_removed: {
    title: "Anyị atụkwasịghịzi onye bịanyere aka obi",
    guidance: "Anyị atụkwasịghịzi onye ọrụ ahụike bịanyere aka na ndekọ a obi.",
    action: "Ejila ndekọ a. Jụọ onye ọrịa ahụ ozugbo.",
    provenance: p,
  },
  attester_suspended: {
    title: "Akwụsịtụrụ onye bịanyere aka",
    guidance: "Akwụsịtụrụ onye ọrụ ahụike bịanyere aka na ndekọ a ruo oge ụfọdụ.",
    action: "Jiri ya kpachara anya. Gwa onye ọrịa ahụ kwado isi ihe ndị dị mkpa.",
    provenance: p,
  },
  expired: {
    title: "Ndekọ ahụ etoola oge",
    guidance: "Ndekọ a agafeela oge anyị kwere. Ọ nwere ike ịbụ nke ochie.",
    action: "Jiri ya kpachara anya. Gwa onye ọrịa ahụ kwado isi ihe ndị dị mkpa.",
    provenance: p,
  },
  verified: {
    title: "Elelela kaadị ahụ",
    guidance: "Onye ọrụ ahụike anyị tụkwasịrị obi bịanyere aka na ndekọ a.",
    action: "Ị nwere ike iji ndekọ a.",
    provenance: p,
  },
};
