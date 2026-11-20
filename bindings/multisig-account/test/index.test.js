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

  it("exposes context-policy methods on the client prototype", () => {
    expect(Client.prototype.set_context_policy).toBeTypeOf("function");
    expect(Client.prototype.remove_context_policy).toBeTypeOf("function");
    expect(Client.prototype.get_context_policy).toBeTypeOf("function");
    expect(Client.prototype.get_default_policy).toBeTypeOf("function");
    expect(Client.prototype.set_default_policy).toBeTypeOf("function");
  });

  it("exposes the atomic replace_signer method on the client prototype", () => {
    expect(Client.prototype.replace_signer).toBeTypeOf("function");
  });

  it("maps SignerReplaced to an error/event code", () => {
    const entry = Object.values(Errors).find(
      (e) => e && e.message === "SignerReplaced",
    );
    expect(entry).toBeDefined();
  });

  it("maps context-policy related errors", () => {
    const messages = Object.values(Errors)
      .filter((e) => e && typeof e.message === "string")
      .map((e) => e.message);
    expect(messages).toContain("DuplicateSigner");
    expect(messages).toContain("UnknownContextPolicy");
  });
});