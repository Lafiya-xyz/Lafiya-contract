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

  it("exposes the policy-based constructor variant", () => {
    const proto = Client.prototype;
    const hasPolicyCtor =
      typeof proto.constructor_with_policy === "function" ||
      typeof proto.constructorWithPolicy === "function" ||
      typeof proto.constructor_policy === "function";
    expect(hasPolicyCtor).toBe(true);
  });

  it("marks set_signers as deprecated in JSDoc", () => {
    const source = Client.prototype.set_signers.toString();
    expect(source).toMatch(/@deprecated/);
  });

  it("exposes the guarded self-upgrade methods", () => {
    const proto = Client.prototype;
    expect(typeof proto.upgrade).toBe("function");
    expect(typeof proto.cancel_upgrade).toBe("function");
    expect(typeof proto.get_pending_upgrade).toBe("function");
  });
});