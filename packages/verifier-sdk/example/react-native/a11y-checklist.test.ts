import { expect, test } from "vitest";
import { checkVerdictStyles } from "./a11y-checklist.js";
import { VERDICT_STYLES } from "./styles.js";

test("verdict styles satisfy the responder UX checklist", () => {
  expect(checkVerdictStyles(VERDICT_STYLES)).toEqual([]);
});
