import { canonical, sha256Hex, type SnapshotStore } from "./snapshot.js";

/** One router entry: where a contract currently lives and which interface it speaks. */
export interface RouterEntry {
  contractId: string;
  interfaceVersion: number;
}

/** The router's full state at one sequence number (`entries` keyed by contract name). */
export interface RouterState {
  /** Increments on every admin update; never decreases. */
  seq: number;
  entries: Record<string, RouterEntry>;
}

/** Reads the optional stable-address router (docs/adr/0018-stable-address-registry-router.md). */
export interface RouterReader {
  getState(routerId: string): Promise<RouterState>;
}

export class RouterError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RouterError";
  }
}

interface Remembered {
  state: RouterState;
  /** sha256 over the canonical state; detects a corrupted local cache. */
  hash: string;
}

export interface Resolution extends RouterEntry {
  /** `true` when the router was unreachable and the last-known state was used. */
  cached: boolean;
  seq: number;
}

/**
 * Resolve `name` through the router. The last-known state is remembered in
 * `store` (keyed by router id); a fetched state with a lower `seq` is rejected
 * as a rollback, and an unreachable router falls back to the remembered
 * state. Direct-id mode needs none of this: keep configuring contract ids.
 */
export async function resolveContract(
  reader: RouterReader,
  store: SnapshotStore,
  routerId: string,
  name: string,
  opts: { minInterfaceVersion?: number } = {},
): Promise<Resolution> {
  const raw = await store.get(routerId);
  let known: Remembered | null = null;
  if (raw !== null) {
    try {
      const parsed = JSON.parse(raw) as Remembered;
      if ((await sha256Hex(canonical(parsed.state))) === parsed.hash) known = parsed;
    } catch {
      // A corrupt cache is a miss, never trusted.
    }
  }

  let state: RouterState;
  let cached = false;
  try {
    state = await reader.getState(routerId);
  } catch (err) {
    if (!known) throw new RouterError(`router unreachable and nothing cached: ${String(err)}`);
    state = known.state;
    cached = true;
  }
  if (known && state.seq < known.state.seq) {
    throw new RouterError(`router state rolled back (seq ${state.seq} < ${known.state.seq})`);
  }
  if (!cached) {
    const remembered: Remembered = { state, hash: await sha256Hex(canonical(state)) };
    await store.put(routerId, JSON.stringify(remembered));
  }

  const entry = state.entries[name];
  if (!entry) throw new RouterError(`router has no entry for "${name}"`);
  if (opts.minInterfaceVersion !== undefined && entry.interfaceVersion < opts.minInterfaceVersion) {
    throw new RouterError(
      `"${name}" speaks interface v${entry.interfaceVersion}, need >= v${opts.minInterfaceVersion}`,
    );
  }
  return { ...entry, cached, seq: state.seq };
}
