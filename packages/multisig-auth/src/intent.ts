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

/** A signer policy as stored on-chain: threshold plus optional signer set. */
export interface SignerPolicy {
  threshold: number;
  /** Signer strkeys (G.../C...) allowed to satisfy the threshold. */
  signers: string[];
}

/** Policy override keyed by `(contract_address, fn_name)`. */
export interface ContextPolicy {
  contract: string;
  function: string;
  policy: SignerPolicy;
}

/** The policy that applies to one context in the invocation tree. */
export interface ContextRequirement {
  contract: string;
  function: string;
  /** True when an explicit `(contract, function)` override matched. */
  matched: boolean;
  policy: SignerPolicy;
}

export interface RequiredSigners {
  /** Strictest policy across every context in the tree. */
  policy: SignerPolicy;
  /** Signers that must approve, i.e. `policy.signers` (may be empty if unconstrained). */
  signers: string[];
  /** Per-context effective policy, in tree order. */
  contexts: ContextRequirement[];
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

function flattenInvocation(invocation: DecodedInvocation): DecodedInvocation[] {
  return [invocation, ...invocation.subInvocations.flatMap(flattenInvocation)];
}

/**
 * Pick the strictest of two policies: higher threshold wins; on a tie the
 * policy with fewer allowed signers is stricter (a smaller set is harder to
 * satisfy). Ties beyond that keep `a` for determinism.
 */
function strictest(a: SignerPolicy, b: SignerPolicy): SignerPolicy {
  if (b.threshold !== a.threshold) return b.threshold > a.threshold ? b : a;
  if (b.signers.length !== a.signers.length) return b.signers.length < a.signers.length ? b : a;
  return a;
}

/**
 * Compute the signer set required to authorize `invocation` given the
 * account's `contextPolicies` and its `defaultPolicy`.
 *
 * Every context in the tree is resolved: an explicit `(contract, function)`
 * override wins, otherwise the default policy applies. Contract-creation
 * contexts (`CreateContractHostFn` / `CreateContractWithCtorHostFn`) and any
 * other non-contract-call function never match an override and therefore fall
 * back to the default, which callers should configure as the strictest policy.
 * The strictest resolved policy across all contexts is the one the quorum must
 * satisfy.
 */
export function requiredSigners(
  invocation: DecodedInvocation,
  contextPolicies: ContextPolicy[],
  defaultPolicy: SignerPolicy,
): RequiredSigners {
  const contexts: ContextRequirement[] = flattenInvocation(invocation).map((node) => {
    const override = contextPolicies.find(
      (p) => p.contract === node.contract && p.function === node.function,
    );
    return {
      contract: node.contract,
      function: node.function,
      matched: override !== undefined,
      policy: override ? override.policy : defaultPolicy,
    };
  });

  const policy = contexts.reduce<SignerPolicy>(
    (acc, ctx) => strictest(acc, ctx.policy),
    defaultPolicy,
  );
  return { policy, signers: [...policy.signers], contexts };
}
