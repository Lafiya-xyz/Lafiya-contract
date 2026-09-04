import { describe, expect, it } from "vitest";
import { Client } from "../dist/index.js";

const EXPECTED_METHODS = [
  "initialize",
  "is_attester",
  "add_attester",
  "remove_attester",
  "get_attester_info",
  "add_attester_with_info",
];

describe("attester-registry generated bindings", () => {
  const client = new Client({
    contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    networkPassphrase: "Test SDF Network ; September 2015",
    rpcUrl: "https://localhost:1",
  });

  it("exports a Client constructor", () => {
    expect(Client).toBeTypeOf("function");
  });

  for (const method of EXPECTED_METHODS) {
    it(`exposes a "${method}" client method`, () => {
      expect(client[method]).toBeTypeOf("function");
    });
  }
});
