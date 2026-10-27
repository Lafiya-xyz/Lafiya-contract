import type { LocaleCatalog, Provenance } from "../i18n.js";

// Hausa draft. Not for clinical use until a native-speaking clinical
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
    title: "Ba katin Lafiya ba ne",
    guidance: "Wannan lambar ba katin lafiya na Lafiya ba ce. Kada a yi amfani da ita wajen magani.",
    action: "Sake duba katin, ko a nemi wata shaida.",
    provenance: p,
  },
  untrusted_contract: {
    title: "Ba a amince da katin ba",
    guidance: "Wannan katin ya fito daga wurin da ba mu amince da shi ba. Kada a yi amfani da shi wajen magani.",
    action: "Ɗauki mara lafiyan kamar ba shi da bayanai.",
    provenance: p,
  },
  commitment_mismatch: {
    title: "Bayanan ba su dace ba",
    guidance: "Bayanan da aka nuna ba su ne na kan katin ba. Wataƙila an canza su.",
    action: "Kada a yi amfani da su. Tambayi mara lafiyan kai tsaye.",
    provenance: p,
  },
  unavailable: {
    title: "Ba a iya dubawa ba",
    guidance: "Ba mu iya duba katin yanzu ba. Ba yana nufin yana da kyau ko ba shi da kyau ba.",
    action: "Sake gwadawa. Idan bai yi ba, tambayi mara lafiyan kai tsaye.",
    provenance: p,
  },
  revoked: {
    title: "An soke katin",
    guidance: "An soke waɗannan bayanan. Kada a yi amfani da su wajen magani.",
    action: "Tambayi mara lafiyan kai tsaye.",
    provenance: p,
  },
  not_found: {
    title: "Ba a sami bayanai ba",
    guidance: "Babu ma'aikacin lafiya da ya sa hannu kan waɗannan bayanan.",
    action: "Kada a yi amfani da su. Tambayi mara lafiyan kai tsaye.",
    provenance: p,
  },
  superseded: {
    title: "Tsohon kati",
    guidance: "Akwai sababbin bayanai na wannan mara lafiyan. Wataƙila wannan katin ya tsufa.",
    action: "Yi amfani da shi a hankali. Nemi sabon katin.",
    provenance: p,
  },
  attester_removed: {
    title: "Ba a amince da mai sa hannu ba",
    guidance: "Ba a ƙara amincewa da ma'aikacin lafiyan da ya sa hannu ba.",
    action: "Kada a yi amfani da su. Tambayi mara lafiyan kai tsaye.",
    provenance: p,
  },
  attester_suspended: {
    title: "An dakatar da mai sa hannu",
    guidance: "An dakatar da ma'aikacin lafiyan da ya sa hannu na ɗan lokaci.",
    action: "Yi amfani da shi a hankali. Tabbatar da muhimman abubuwa da mara lafiyan.",
    provenance: p,
  },
  expired: {
    title: "Bayanan sun tsufa",
    guidance: "Waɗannan bayanan sun wuce lokacin da aka yarda. Wataƙila sun tsufa.",
    action: "Yi amfani da su a hankali. Tabbatar da muhimman abubuwa da mara lafiyan.",
    provenance: p,
  },
  verified: {
    title: "An tabbatar da katin",
    guidance: "Ma'aikacin lafiya da aka amince da shi ne ya sa hannu.",
    action: "Za ka iya amfani da waɗannan bayanan.",
    provenance: p,
  },
};
