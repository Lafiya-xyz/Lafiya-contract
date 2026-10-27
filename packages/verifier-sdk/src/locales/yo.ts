import type { LocaleCatalog, Provenance } from "../i18n.js";

// Yoruba draft. Not for clinical use until a native-speaking clinical
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
    title: "Kì í ṣe káàdì Lafiya",
    guidance: "Kóòdù yìí kì í ṣe káàdì ìlera Lafiya. Má ṣe lò ó fún ìtọ́jú.",
    action: "Tún ka káàdì náà, tàbí béèrè fún ìdánimọ̀ míràn.",
    provenance: p,
  },
  untrusted_contract: {
    title: "A kò gbẹ́kẹ̀lé káàdì yìí",
    guidance: "Káàdì yìí wá láti ibi tí a kò gbẹ́kẹ̀lé. Má ṣe lò ó fún ìtọ́jú.",
    action: "Ṣe bí ẹni pé aláìsàn náà kò ní àkọsílẹ̀.",
    provenance: p,
  },
  commitment_mismatch: {
    title: "Àkọsílẹ̀ kò bá a mu",
    guidance: "Àkọsílẹ̀ tí a fihàn kì í ṣe èyí tó wà lórí káàdì. Wọ́n lè ti yí i padà.",
    action: "Má ṣe lo àkọsílẹ̀ yìí. Béèrè lọ́wọ́ aláìsàn fúnra rẹ̀.",
    provenance: p,
  },
  unavailable: {
    title: "A kò lè ṣàyẹ̀wò rẹ̀",
    guidance: "A kò lè ṣàyẹ̀wò káàdì náà báyìí. Èyí kò túmọ̀ sí pé ó dára tàbí kò dára.",
    action: "Gbìyànjú lẹ́ẹ̀kan sí i. Tí kò bá ṣiṣẹ́, béèrè lọ́wọ́ aláìsàn fúnra rẹ̀.",
    provenance: p,
  },
  revoked: {
    title: "Wọ́n ti fagilé káàdì",
    guidance: "Wọ́n ti fagilé àkọsílẹ̀ yìí. Má ṣe lò ó fún ìtọ́jú.",
    action: "Béèrè lọ́wọ́ aláìsàn fúnra rẹ̀.",
    provenance: p,
  },
  not_found: {
    title: "A kò rí àkọsílẹ̀",
    guidance: "Kò sí òṣìṣẹ́ ìlera tó fọwọ́ sí àkọsílẹ̀ yìí.",
    action: "Má ṣe lo àkọsílẹ̀ yìí. Béèrè lọ́wọ́ aláìsàn fúnra rẹ̀.",
    provenance: p,
  },
  superseded: {
    title: "Káàdì àtijọ́",
    guidance: "Àkọsílẹ̀ tuntun wà fún aláìsàn yìí. Káàdì yìí lè ti di àtijọ́.",
    action: "Lò ó pẹ̀lú ìṣọ́ra. Béèrè fún káàdì tuntun jùlọ.",
    provenance: p,
  },
  attester_removed: {
    title: "A kò gbẹ́kẹ̀lé ẹni tó fọwọ́ sí i mọ́",
    guidance: "A kò gbẹ́kẹ̀lé òṣìṣẹ́ ìlera tó fọwọ́ sí àkọsílẹ̀ yìí mọ́.",
    action: "Má ṣe lo àkọsílẹ̀ yìí. Béèrè lọ́wọ́ aláìsàn fúnra rẹ̀.",
    provenance: p,
  },
  attester_suspended: {
    title: "Wọ́n ti dá ẹni tó fọwọ́ sí i dúró",
    guidance: "Wọ́n ti dá òṣìṣẹ́ ìlera tó fọwọ́ sí àkọsílẹ̀ yìí dúró fún ìgbà díẹ̀.",
    action: "Lò ó pẹ̀lú ìṣọ́ra. Jẹ́rìí àwọn kókó pàtàkì pẹ̀lú aláìsàn.",
    provenance: p,
  },
  expired: {
    title: "Àkọsílẹ̀ ti pẹ́ jù",
    guidance: "Àkọsílẹ̀ yìí ti kọjá àkókò tí a gbà láàyè. Ó lè ti di àtijọ́.",
    action: "Lò ó pẹ̀lú ìṣọ́ra. Jẹ́rìí àwọn kókó pàtàkì pẹ̀lú aláìsàn.",
    provenance: p,
  },
  verified: {
    title: "A ti ṣàyẹ̀wò káàdì",
    guidance: "Òṣìṣẹ́ ìlera tí a gbẹ́kẹ̀lé ló fọwọ́ sí àkọsílẹ̀ yìí.",
    action: "O lè lo àkọsílẹ̀ yìí.",
    provenance: p,
  },
};
