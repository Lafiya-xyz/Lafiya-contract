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
