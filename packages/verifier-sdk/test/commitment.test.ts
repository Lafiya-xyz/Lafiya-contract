import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { commitV1, encodePayload, toHex, type FieldValue } from "../src/index.js";

interface Vector {
  name: string;
  fields: { kind: string; value?: string | boolean }[];
  payload_hex: string;
  commitment_hex: string;
}

const vectors: Vector[] = JSON.parse(
  readFileSync(
    new URL("../../../crates/lafiya-commitment/vectors/lrc1-test-vectors.json", import.meta.url),
    "utf8",
  ),
);

function field(f: Vector["fields"][number]): FieldValue {
  switch (f.kind) {
    case "text":
      return { kind: "text", value: f.value as string };
    case "int64":
      return { kind: "int64", value: BigInt(f.value as string) };
    case "bool":
      return { kind: "bool", value: f.value as boolean };
    case "bytes":
      return { kind: "bytes", value: Uint8Array.from(Buffer.from(f.value as string, "hex")) };
    default:
      return { kind: f.kind as "absent" | "null" };
  }
}

describe("LRC-1 commitment", () => {
  it("has vectors", () => expect(vectors.length).toBeGreaterThan(0));

  for (const v of vectors) {
    it(`matches the shared vector "${v.name}"`, async () => {
      const fields = v.fields.map(field);
      expect(toHex(encodePayload(fields))).toBe(v.payload_hex);
      expect(await commitV1(fields)).toBe(v.commitment_hex);
    });
  }

  it("normalizes text to NFC", async () => {
    const composed = await commitV1([{ kind: "text", value: "café" }]);
    const decomposed = await commitV1([{ kind: "text", value: "café" }]);
    expect(composed).toBe(decomposed);
  });
});
