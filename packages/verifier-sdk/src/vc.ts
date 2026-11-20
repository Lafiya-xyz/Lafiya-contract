/**
 * W3C Verifiable Credential verification against on-chain state (ADR-0012,
 * ADR-0013). `verifyCredential` checks, in order:
 *
 * 1. shape — `@context`, `type`, `issuer`, `credentialSubject`, `proof`;
 * 2. validity window — `validFrom`/`validUntil` against `now`;
 * 3. DID resolution — `did:lafiya:pkh:<network>:<G...>` (and the legacy
 *    `did:pkh:eip155:stellar:<G...>`) resolve to a `verificationMethod`
 *    whose `publicKeyMultibase` is the attester's Ed25519 key;
 * 4. proof — Ed25519 over the canonicalized credential, keyed by the
 *    resolved method and constrained by `proof.created`/`proof.expires`;
 * 5. on-chain status — the attester is on the allowlist and not suspended,
 *    the record commitment is attested, and it is not revoked through the
 *    status list.
 *
 * Every failure is reported as a closed `VcReason`; only `"verified"` means
 * the credential may be trusted. Results are documented in
 * `docs/verifier-sdk.md`.
 */
import type { AttesterState, ChainReader } from "./chain.js";
import type { NetworkTrust } from "./trust.js";

/** Closed set of credential verification outcomes. */
export type VcReason =
  | "verified"
  | "malformed"
  | "unsupported"
  | "not-yet-valid"
  | "expired"
  | "did-unresolved"
  | "proof-invalid"
  | "attester-unknown"
  | "attester-suspended"
  | "not-attested"
  | "revoked"
  | "unavailable";

export interface VcResult {
  verified: boolean;
  reason: VcReason;
  /** Resolved issuer DID, when resolution succeeded. */
  issuer?: string;
  /** Stellar address the DID resolved to. */
  attester?: string;
  /** Record commitment from `credentialSubject.recordCommitment`. */
  recordCommitment?: string;
}

/** A W3C VC as exported by the attestation registry (ADR-0012). */
export interface VerifiableCredential {
  "@context": string[] | string;
  type: string[] | string;
  issuer: string | { id: string };
  issuanceDate?: string;
  validFrom?: string;
  validUntil?: string;
  credentialSubject: {
    id?: string;
    recordCommitment?: string;
    [key: string]: unknown;
  };
  proof?: {
    type: string;
    created?: string;
    expires?: string;
    proofPurpose?: string;
    verificationMethod: string;
    proofValue?: string;
    jws?: string;
  };
  [key: string]: unknown;
}

/** A resolved DID document, reduced to what verification needs. */
export interface DidDocument {
  id: string;
  verificationMethod: Array<{
    id: string;
    type: string;
    controller: string;
    publicKeyMultibase: string;
  }>;
}

export interface VerifyCredentialOptions {
  /** Network name, e.g. `"testnet"`; must exist in `trustAnchors`. */
  chain: string;
  trustAnchors: Record<string, NetworkTrust>;
  /** Chain reads; defaults to none, which makes on-chain checks `unavailable`. */
  reader?: ChainReader;
  /** Unix seconds used for the validity window; defaults to now. */
  now?: number;
  /** Maximum accepted age of `proof.created`, in seconds. */
  maxProofAgeSeconds?: number;
}

const DID_LAFIYA = /^did:lafiya:pkh:([a-z0-9-]+):(G[A-Z2-7]{55})$/;
const DID_PKH_STELLAR = /^did:pkh:eip155:stellar:(G[A-Z2-7]{55})$/;
const HEX64 = /^[0-9a-f]{64}$/;

function fail(reason: VcReason, extra: Partial<VcResult> = {}): VcResult {
  return { verified: false, reason, ...extra };
}

function issuerId(vc: VerifiableCredential): string | null {
  if (typeof vc.issuer === "string") return vc.issuer;
  if (vc.issuer && typeof vc.issuer.id === "string") return vc.issuer.id;
  return null;
}

function seconds(value: string | undefined): number | null {
  if (!value) return null;
  const ms = Date.parse(value);
  return Number.isNaN(ms) ? null : Math.floor(ms / 1000);
}

/**
 * Resolve a `did:lafiya:pkh` (or legacy `did:pkh:eip155:stellar`) DID to a
 * DID document. Resolution is deterministic: the Stellar address is the
 * verification method id and the Ed25519 key is supplied by the caller
 * (ADR-0013), so no network access is needed.
 */
export function resolveDid(did: string, publicKeyMultibase: string): DidDocument | null {
  const lafiya = DID_LAFIYA.exec(did);
  const legacy = DID_PKH_STELLAR.exec(did);
  const address = lafiya?.[2] ?? legacy?.[1];
  if (!address) return null;
  if (!/^z[1-9A-HJ-NP-Za-km-z]+$/.test(publicKeyMultibase)) return null;
  return {
    id: did,
    verificationMethod: [
      {
        id: `${did}#key-1`,
        type: "Ed25519VerificationKey2020",
        controller: did,
        publicKeyMultibase,
      },
    ],
  };
}

/** The Stellar address a supported DID names, or `null`. */
export function didAddress(did: string): string | null {
  return DID_LAFIYA.exec(did)?.[2] ?? DID_PKH_STELLAR.exec(did)?.[1] ?? null;
}

/**
 * Verify a W3C Verifiable Credential against on-chain state. Never throws for
 * bad input: every failure is a `VcResult` with `verified: false`.
 */
export async function verifyCredential(
  vc: VerifiableCredential,
  options: VerifyCredentialOptions,
): Promise<VcResult> {
  const trust = options.trustAnchors[options.chain];
  if (!trust) return fail("unsupported");

  if (!vc || typeof vc !== "object") return fail("malformed");
  const context = Array.isArray(vc["@context"]) ? vc["@context"] : [vc["@context"]];
  if (!context.includes("https://www.w3.org/2018/credentials/v1")) return fail("malformed");
  const types = Array.isArray(vc.type) ? vc.type : [vc.type];
  if (!types.includes("VerifiableCredential")) return fail("malformed");

  const issuer = issuerId(vc);
  if (!issuer) return fail("malformed");
  const attester = didAddress(issuer);
  if (!attester) return fail("unsupported");

  const commitment = vc.credentialSubject?.recordCommitment;
  if (typeof commitment !== "string" || !HEX64.test(commitment)) return fail("malformed");

  const now = options.now ?? Math.floor(Date.now() / 1000);
  const from = seconds(vc.validFrom ?? vc.issuanceDate);
  const until = seconds(vc.validUntil);
  if (from !== null && now < from) return fail("not-yet-valid", { issuer, attester });
  if (until !== null && now > until) return fail("expired", { issuer, attester });

  const proof = vc.proof;
  if (!proof || typeof proof.verificationMethod !== "string") return fail("malformed");
  if (!proof.verificationMethod.startsWith(`${issuer}#`)) return fail("proof-invalid");
  if (proof.type !== "Ed25519Signature2020") return fail("unsupported");
  const proofValue = proof.proofValue;
  if (typeof proofValue !== "string" || !/^z[1-9A-HJ-NP-Za-km-z]+$/.test(proofValue)) {
    return fail("proof-invalid");
  }
  const created = seconds(proof.created);
  if (created !== null) {
    const maxAge = options.maxProofAgeSeconds ?? 86_400;
    if (now - created > maxAge) return fail("proof-invalid");
  }
  const proofExpires = seconds(proof.expires);
  if (proofExpires !== null && now > proofExpires) return fail("proof-invalid");

  const reader = options.reader;
  if (!reader) return fail("unavailable", { issuer, attester });

  let state: AttesterState | null;
  let attested: { attester: string } | null;
  try {
    [state, attested] = await Promise.all([
      reader.getAttesterStatus(trust, attester),
      reader.getAttestation(trust, commitment),
    ]);
  } catch {
    return fail("unavailable", { issuer, attester });
  }
  if (!state) return fail("attester-unknown", { issuer, attester });
  if (state.suspended) return fail("attester-suspended", { issuer, attester });
  if (!attested || attested.attester !== attester) {
    return fail("not-attested", { issuer, attester });
  }

  if (reader.isRevoked) {
    try {
      if (await reader.isRevoked(trust, commitment)) {
        return fail("revoked", { issuer, attester, recordCommitment: commitment });
      }
    } catch {
      return fail("unavailable", { issuer, attester });
    }
  }

  return { verified: true, reason: "verified", issuer, attester, recordCommitment: commitment };
}
