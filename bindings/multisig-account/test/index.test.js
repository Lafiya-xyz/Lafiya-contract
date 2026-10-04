import { describe, expect, it } from "vitest";
import { Client, Errors } from "../dist/index.js";

describe("multisig-account generated bindings", () => {
  it("exports a Client constructor", () => {
    expect(Client).toBeTypeOf("function");
  });

  it("exposes a static deploy helper for the constructor", () => {
    expect(Client.deploy).toBeTypeOf("function");
  });

  it("maps BadSignatureOrder to error code 4", () => {
    expect(Errors[4].message).toBe("BadSignatureOrder");
  });
});
