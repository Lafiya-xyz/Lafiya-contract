import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { evaluateModel, MODEL_VERDICTS, MODEL_VERSION, type ModelInput } from "../src/index.js";

const table = JSON.parse(
  readFileSync(new URL("../../../docs/specs/verification-model.json", import.meta.url), "utf8"),
) as {
  model_version: string;
  verdicts: string[];
  default_input: ModelInput;
  cases: { id: string; input: Partial<ModelInput>; verdict: string; qualifier?: string }[];
};

describe("verification model truth table", () => {
  it("implements the current model version and verdict set", () => {
    expect(MODEL_VERSION).toBe(table.model_version);
    expect([...MODEL_VERDICTS]).toEqual(table.verdicts);
  });

  it.each(table.cases)("$id", (c) => {
    const result = evaluateModel({ ...table.default_input, ...c.input });
    expect(result.verdict).toBe(c.verdict);
    expect(result.qualifier).toBe(c.qualifier ?? null);
  });
});
