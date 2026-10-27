/**
 * Human-readable view of what a `SorobanAuthorizationEntry` authorizes, for
 * the review step a signer must see before approving with a passkey.
 * `multisig-account` does not scope what a quorum approves (ADR-0007), so a
 * signer is approving the whole invocation tree, and the passkey prompt
 * itself only shows the relying party, never the call. Mirrors the CLI's
 * `lafiya-cli auth decode` (crates/lafiya-cli/src/auth_decode.rs).
 */
import { Buffer } from "buffer";
import { Address, scValToNative, xdr } from "@stellar/stellar-sdk";

/** One node of the decoded invocation tree. */
export interface DecodedInvocation {
  /** Contract strkey (C...), or a label for non-contract-call functions. */
  contract: string;
  function: string;
  /** Arguments converted with `scValToNative`, then JSON-safe stringified. */
  args: string[];
  subInvocations: DecodedInvocation[];
}

export interface DecodedIntent {
  /** Account whose authorization is requested (the multisig contract). */
  signer: string | null;
  validUntilLedger: number;
  invocation: DecodedInvocation;
}

function stringify(value: unknown): string {
  return JSON.stringify(value, (_key, v: unknown) => {
    if (typeof v === "bigint") return v.toString();
    if (v instanceof Uint8Array) return Buffer.from(v).toString("hex");
    return v;
  });
}

function decodeInvocation(invocation: xdr.SorobanAuthorizedInvocation): DecodedInvocation {
  const fn = invocation.function();
  const subInvocations = invocation.subInvocations().map(decodeInvocation);
  if (
    fn.switch().value ===
    xdr.SorobanAuthorizedFunctionType.sorobanAuthorizedFunctionTypeContractFn().value
  ) {
    const call = fn.contractFn();
    return {
      contract: Address.fromScAddress(call.contractAddress()).toString(),
      function: call.functionName().toString(),
      args: call.args().map((arg) => stringify(scValToNative(arg))),
      subInvocations,
    };
  }
  return { contract: fn.switch().name, function: fn.switch().name, args: [], subInvocations };
}

/** Decode `entry` into the call tree a signer is approving. */
export function decodeIntent(
  entry: xdr.SorobanAuthorizationEntry,
  validUntilLedger: number,
): DecodedIntent {
  const credentials = entry.credentials();
  const signer =
    credentials.switch().value === xdr.SorobanCredentialsType.sorobanCredentialsAddress().value
      ? Address.fromScAddress(credentials.address().address()).toString()
      : null;
  return { signer, validUntilLedger, invocation: decodeInvocation(entry.rootInvocation()) };
}
