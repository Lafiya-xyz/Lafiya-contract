/** What the verifier trusts for one network. Never taken from a QR code. */
export interface NetworkTrust {
  networkPassphrase: string;
  rpcUrl: string;
  attestationRegistry: string;
  attesterRegistry: string;
}

/** Trust anchors keyed by network name (`testnet`, `mainnet`, ...). */
export type TrustAnchors = Record<string, NetworkTrust>;

export class TrustError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TrustError";
  }
}

const CONTRACT_ID = /^C[A-Z2-7]{55}$/;

function checked(network: string, t: Partial<NetworkTrust>): NetworkTrust {
  for (const key of ["networkPassphrase", "rpcUrl"] as const) {
    if (!t[key]) throw new TrustError(`${network}: missing ${key}`);
  }
  for (const key of ["attestationRegistry", "attesterRegistry"] as const) {
    if (!CONTRACT_ID.test(t[key] ?? "")) {
      throw new TrustError(`${network}: ${key} must be a C... contract id`);
    }
  }
  return t as NetworkTrust;
}

export type TrustSource =
  | { config: Record<string, Partial<NetworkTrust>> }
  | {
      /** Home domain publishing `/.well-known/stellar.toml` (SEP-1). */
      domain: string;
      network: string;
      rpcUrl: string;
      fetch?: typeof globalThis.fetch;
    };

/**
 * Load trust anchors from explicit configuration, or discover them from a
 * SEP-1 `stellar.toml` that publishes `NETWORK_PASSPHRASE`,
 * `LAFIYA_ATTESTATION_REGISTRY`, and `LAFIYA_ATTESTER_REGISTRY`.
 */
export async function loadTrustAnchors(source: TrustSource): Promise<TrustAnchors> {
  if ("config" in source) {
    return Object.fromEntries(
      Object.entries(source.config).map(([name, t]) => [name, checked(name, t)]),
    );
  }
  const fetchImpl = source.fetch ?? globalThis.fetch;
  const url = `https://${source.domain}/.well-known/stellar.toml`;
  const res = await fetchImpl(url);
  if (!res.ok) throw new TrustError(`${url}: HTTP ${res.status}`);
  const toml = parseTopLevelToml(await res.text());
  return {
    [source.network]: checked(source.network, {
      networkPassphrase: toml.NETWORK_PASSPHRASE,
      rpcUrl: source.rpcUrl,
      attestationRegistry: toml.LAFIYA_ATTESTATION_REGISTRY,
      attesterRegistry: toml.LAFIYA_ATTESTER_REGISTRY,
    }),
  };
}

/** Top-level `KEY = "value"` pairs of a TOML file (tables are ignored). */
function parseTopLevelToml(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (trimmed.startsWith("[")) break;
    const m = /^([A-Za-z0-9_]+)\s*=\s*"([^"]*)"/.exec(trimmed);
    if (m) out[m[1]] = m[2];
  }
  return out;
}

/**
 * A signed trust-anchor document published by a foreign deployment's
 * governance. The signature covers the canonical JSON of every field except
 * `signature` (see `canonicalAnchorPayload`).
 */
export interface FederationTrustAnchor {
  /** Jurisdiction identifier, e.g. `ng`, `ke`, `gh`. */
  jurisdiction: string;
  /** Monotonic sequence; a verifier must reject anchors older than the last seen. */
  sequence: number;
  /** ISO-8601 validity window. */
  validFrom: string;
  validUntil: string;
  networkPassphrase: string;
  rpcUrl: string;
  attestationRegistry: string;
  attesterRegistry: string;
  /** Governance public key(s) that signed this document (G... ed25519). */
  signers: string[];
  /** Hex-encoded ed25519 signature over `canonicalAnchorPayload(anchor)`. */
  signature: string;
}

/** Pinned governance keys per jurisdiction, configured out-of-band. */
export type PinnedGovernanceKeys = Record<string, string[]>;

export type FederationTrustSource = {
  /** Base URL of the foreign deployment, e.g. `https://ke.example.org`. */
  baseUrl: string;
  /** Pinned governance keys for the expected jurisdiction. */
  pinnedKeys: PinnedGovernanceKeys;
  /** Highest anchor sequence already accepted for this jurisdiction. */
  lastSeenSequence?: number;
  /** Injectable clock for tests. */
  now?: () => Date;
  fetch?: typeof globalThis.fetch;
};

/** Deterministic JSON of the anchor fields covered by the signature. */
export function canonicalAnchorPayload(a: FederationTrustAnchor): string {
  const { signature: _sig, ...signed } = a;
  const keys = Object.keys(signed).sort();
  return JSON.stringify(
    Object.fromEntries(keys.map((k) => [k, (signed as Record<string, unknown>)[k]])),
  );
}

/**
 * Fetch and verify a foreign deployment's signed trust-anchor document against
 * pinned governance keys. Throws `TrustError` on any failure; the caller maps
 * these to `VerificationResult` reasons.
 */
export async function loadFederationAnchors(
  source: FederationTrustSource,
): Promise<FederationTrustAnchor> {
  const fetchImpl = source.fetch ?? globalThis.fetch;
  const url = `${source.baseUrl.replace(/\/$/, "")}/.well-known/lafiya-federation.json`;
  let res: Response;
  try {
    res = await fetchImpl(url);
  } catch (err) {
    throw new TrustError(`${url}: unavailable (${(err as Error).message})`);
  }
  if (!res.ok) throw new TrustError(`${url}: HTTP ${res.status}`);
  const anchor = (await res.json()) as FederationTrustAnchor;

  const pinned = source.pinnedKeys[anchor.jurisdiction];
  if (!pinned || pinned.length === 0) {
    throw new TrustError(`unknown jurisdiction: ${anchor.jurisdiction}`);
  }
  if (source.lastSeenSequence !== undefined && anchor.sequence <= source.lastSeenSequence) {
    throw new TrustError(`stale anchor: sequence ${anchor.sequence} <= ${source.lastSeenSequence}`);
  }
  const now = (source.now ?? (() => new Date()))().getTime();
  if (now < Date.parse(anchor.validFrom) || now > Date.parse(anchor.validUntil)) {
    throw new TrustError(`anchor outside validity window`);
  }
  for (const key of ["attestationRegistry", "attesterRegistry"] as const) {
    if (!CONTRACT_ID.test(anchor[key] ?? "")) {
      throw new TrustError(`${key} must be a C... contract id`);
    }
  }
  if (!anchor.signers.some((s) => pinned.includes(s))) {
    throw new TrustError(`anchor not signed by a pinned governance key`);
  }
  if (!(await verifyAnchorSignature(anchor))) {
    throw new TrustError(`bad anchor signature`);
  }
  return anchor;
}

/** Verify the ed25519 signature over the canonical payload. */
async function verifyAnchorSignature(a: FederationTrustAnchor): Promise<boolean> {
  const signer = a.signers[0];
  if (!signer || !/^G[A-Z2-7]{55}$/.test(signer)) return false;
  const payload = new TextEncoder().encode(canonicalAnchorPayload(a));
  const sig = hexToBytes(a.signature);
  if (!sig) return false;
  try {
    const { StrKey, Keypair } = await import("@stellar/stellar-sdk");
    const raw = StrKey.decodeEd25519PublicKey(signer);
    const kp = Keypair.fromPublicKey(signer);
    // `verify` is exposed on the underlying ed25519 implementation.
    return (kp as unknown as {
      verify: (data: Uint8Array, sig: Uint8Array) => boolean;
    }).verify(payload, sig) && raw.length === 32;
  } catch {
    return false;
  }
}

function hexToBytes(hex: string): Uint8Array | null {
  if (!/^[0-9a-fA-F]*$/.test(hex) || hex.length % 2 !== 0) return null;
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** Same shape as the single-deployment verifier result. */
export interface VerificationResult {
  valid: boolean;
  reason?:
    | "unknown_jurisdiction"
    | "stale_anchor"
    | "bad_signature"
    | "foreign_registry_unavailable"
    | "attestation_not_found"
    | "attester_not_trusted"
    | "attestation_revoked";
}

/** Minimal read-only registry surface used for foreign lookups. */
export interface ForeignRegistryReader {
  getAttestation(id: string): Promise<{ revoked: boolean } | null>;
  getAttesterStatus(attester: string): Promise<{ trusted: boolean } | null>;
}

export type ForeignAttestationQuery = {
  jurisdiction: string;
  attestationId: string;
  attester: string;
  /** Resolves a jurisdiction to its verified anchor (e.g. via `loadFederationAnchors`). */
  resolveAnchor: (jurisdiction: string) => Promise<FederationTrustAnchor>;
  /** Opens a read-only reader against the anchor's registry IDs. */
  openReader: (anchor: FederationTrustAnchor) => ForeignRegistryReader;
};

/**
 * Read-only cross-registry check of an attestation anchored in another
 * deployment. Never writes to the foreign registry.
 */
export async function verifyForeignAttestation(
  query: ForeignAttestationQuery,
): Promise<VerificationResult> {
  let anchor: FederationTrustAnchor;
  try {
    anchor = await query.resolveAnchor(query.jurisdiction);
  } catch (err) {
    const msg = (err as Error).message ?? "";
    if (msg.startsWith("unknown jurisdiction")) return { valid: false, reason: "unknown_jurisdiction" };
    if (msg.startsWith("stale anchor")) return { valid: false, reason: "stale_anchor" };
    if (msg.startsWith("bad anchor signature")) return { valid: false, reason: "bad_signature" };
    return { valid: false, reason: "foreign_registry_unavailable" };
  }

  let reader: ForeignRegistryReader;
  try {
    reader = query.openReader(anchor);
  } catch {
    return { valid: false, reason: "foreign_registry_unavailable" };
  }

  try {
    const attester = await reader.getAttesterStatus(query.attester);
    if (!attester || !attester.trusted) return { valid: false, reason: "attester_not_trusted" };
    const att = await reader.getAttestation(query.attestationId);
    if (!att) return { valid: false, reason: "attestation_not_found" };
    if (att.revoked) return { valid: false, reason: "attestation_revoked" };
    return { valid: true };
  } catch {
    return { valid: false, reason: "foreign_registry_unavailable" };
  }
}
