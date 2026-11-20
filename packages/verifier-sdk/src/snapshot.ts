import { OfflineChainReader, type OfflineBundle } from "./chain.js";

/** Bump when the stored record layout changes; unknown versions read as a cache miss. */
export const SNAPSHOT_SCHEMA = 2;

export class SnapshotError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SnapshotError";
  }
}

/**
 * A sealed {@link OfflineBundle}. `hash` commits to the bundle, the ledger it
 * was read at, the status-list `epoch` the bundle was read for, and the
 * previous snapshot's hash, so a cache cannot be silently rolled back or
 * spliced. `signature` (over `hash`) is optional and checked by a
 * caller-supplied verifier, e.g. the snapshot publisher's key.
 *
 * The `epoch` is embedded so an offline verifier reads a root and its
 * segments as one epoch-consistent pair rather than mixing epochs.
 */
export interface SnapshotEnvelope {
  schema: number;
  bundle: OfflineBundle;
  ledger: number;
  epoch: number;
  prev: string | null;
  hash: string;
  signature?: string;
}

export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    const entries = Object.entries(value).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${canonical(v)}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

export async function sha256Hex(text: string): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

function bodyHash(e: Omit<SnapshotEnvelope, "hash" | "signature">): Promise<string> {
  return sha256Hex(
    canonical({ schema: e.schema, bundle: e.bundle, ledger: e.ledger, epoch: e.epoch, prev: e.prev }),
  );
}

export async function sealSnapshot(
  bundle: OfflineBundle,
  ledger: number,
  epoch: number,
  prev: string | null,
  sign?: (hash: string) => Promise<string> | string,
): Promise<SnapshotEnvelope> {
  if (!Number.isSafeInteger(epoch) || epoch < 0) {
    throw new SnapshotError(`invalid status-list epoch ${epoch}`);
  }
  const body = { schema: SNAPSHOT_SCHEMA, bundle, ledger, epoch, prev };
  const hash = await bodyHash(body);
  return { ...body, hash, ...(sign ? { signature: await sign(hash) } : {}) };
}

export interface OpenOptions {
  /** When set, a signature is required and must pass this check. */
  verifySignature?: (hash: string, signature: string) => Promise<boolean> | boolean;
  /**
   * When set, the snapshot's embedded epoch must match. Use this to pin a
   * verifier to the epoch whose root it already trusts.
   */
  expectedEpoch?: number;
}

/** Check integrity (and signature, if required); throws {@link SnapshotError}. */
export async function openSnapshot(
  envelope: SnapshotEnvelope,
  opts: OpenOptions = {},
): Promise<OfflineChainReader> {
  if (envelope.schema !== SNAPSHOT_SCHEMA) {
    throw new SnapshotError(`unsupported snapshot schema ${envelope.schema}`);
  }
  if (!Number.isSafeInteger(envelope.epoch) || envelope.epoch < 0) {
    throw new SnapshotError(`invalid status-list epoch ${envelope.epoch}`);
  }
  if (opts.expectedEpoch !== undefined && envelope.epoch !== opts.expectedEpoch) {
    throw new SnapshotError(
      `snapshot epoch ${envelope.epoch} does not match expected epoch ${opts.expectedEpoch}`,
    );
  }
  if ((await bodyHash(envelope)) !== envelope.hash) {
    throw new SnapshotError("snapshot hash mismatch (corrupted or tampered)");
  }
  if (opts.verifySignature) {
    if (!envelope.signature || !(await opts.verifySignature(envelope.hash, envelope.signature))) {
      throw new SnapshotError("snapshot signature missing or invalid");
    }
  }
  return new OfflineChainReader(envelope.bundle);
}

/** Pluggable persistence: one serialized envelope per network. */
export interface SnapshotStore {
  get(network: string): Promise<string | null>;
  put(network: string, value: string): Promise<void>;
}

export class MemorySnapshotStore implements SnapshotStore {
  private readonly data = new Map<string, string>();
  async get(network: string) {
    return this.data.get(network) ?? null;
  }
  async put(network: string, value: string) {
    this.data.set(network, value);
  }
}

/** Node: one `<network>.snapshot.json` file per network under `dir`. */
export class FileSnapshotStore implements SnapshotStore {
  constructor(private readonly dir: string) {}

  private async path(network: string) {
    if (!/^[A-Za-z0-9_-]+$/.test(network)) throw new SnapshotError("invalid network name");
    return (await import("node:path")).join(this.dir, `${network}.snapshot.json`);
  }

  async get(network: string) {
    const fs = await import("node:fs/promises");
    try {
      return await fs.readFile(await this.path(network), "utf8");
    } catch (err) {
      if ((err as { code?: string }).code === "ENOENT") return null;
      throw err;
    }
  }

  async put(network: string, value: string) {
    const fs = await import("node:fs/promises");
    await fs.mkdir(this.dir, { recursive: true });
    await fs.writeFile(await this.path(network), value, "utf8");
  }
}

/** Browser: a single object store in an IndexedDB database. */
export class IndexedDbSnapshotStore implements SnapshotStore {
  private db?: Promise<IDBDatabase>;

  constructor(
    private readonly factory: IDBFactory = globalThis.indexedDB,
    private readonly name = "lafiya-verifier",
  ) {}

  private open(): Promise<IDBDatabase> {
    this.db ??= new Promise((resolve, reject) => {
      const req = this.factory.open(this.name, SNAPSHOT_SCHEMA);
      req.onupgradeneeded = () => req.result.createObjectStore("snapshots");
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
    return this.db;
  }

  private async request<T>(mode: IDBTransactionMode, run: (s: IDBObjectStore) => IDBRequest<T>) {
    const store = (await this.open()).transaction("snapshots", mode).objectStore("snapshots");
    return new Promise<T>((resolve, reject) => {
      const req = run(store);
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
  }

  async get(network: string) {
    return ((await this.request("readonly", (s) => s.get(network))) as string | undefined) ?? null;
  }

  async put(network: string, value: string) {
    await this.request("readwrite", (s) => s.put(value, network));
  }
}

/** Seals, stores, and reloads snapshots as a hash chain. */
export class SnapshotCache {
  constructor(
    private readonly store: SnapshotStore,
    private readonly opts: OpenOptions & { sign?: (hash: string) => Promise<string> | string } = {},
  ) {}

  private async current(network: string): Promise<SnapshotEnvelope | null> {
    const raw = await this.store.get(network);
    if (raw === null) return null;
    try {
      const parsed = JSON.parse(raw) as SnapshotEnvelope;
      return parsed.schema === SNAPSHOT_SCHEMA ? parsed : null;
    } catch {
      return null;
    }
  }

  /**
   * Persist `bundle` for status-list `epoch`; refuses to move the cache back
   * to an older ledger or to an older epoch.
   */
  async save(bundle: OfflineBundle, ledger: number, epoch: number): Promise<SnapshotEnvelope> {
    const prev = await this.current(bundle.network);
    if (prev && ledger < prev.ledger) {
      throw new SnapshotError(`ledger ${ledger} is older than cached ledger ${prev.ledger}`);
    }
    if (prev && epoch < prev.epoch) {
      throw new SnapshotError(`epoch ${epoch} is older than cached epoch ${prev.epoch}`);
    }
    const envelope = await sealSnapshot(bundle, ledger, epoch, prev?.hash ?? null, this.opts.sign);
    await this.store.put(bundle.network, JSON.stringify(envelope));
    return envelope;
  }

  /** The cached snapshot as a reader, or `null` if absent, unreadable, or failing checks. */
  async load(network: string): Promise<OfflineChainReader | null> {
    const envelope = await this.current(network);
    if (!envelope || envelope.bundle.network !== network) return null;
    try {
      return await openSnapshot(envelope, this.opts);
    } catch {
      return null;
    }
  }

  /**
   * The cached snapshot as an epoch-consistent root/segment pair, or `null`
   * if absent, unreadable, or failing checks. The reader is bound to the
   * snapshot's embedded epoch, so callers must read `root(epoch)` first and
   * then segments for that same epoch.
   */
  async loadEpoch(
    network: string,
  ): Promise<{ epoch: number; reader: OfflineChainReader } | null> {
    const envelope = await this.current(network);
    if (!envelope || envelope.bundle.network !== network) return null;
    try {
      const reader = await openSnapshot(envelope, this.opts);
      return { epoch: envelope.epoch, reader };
    } catch {
      return null;
    }
  }
}
