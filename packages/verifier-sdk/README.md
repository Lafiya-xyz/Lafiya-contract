# @lafiya/verifier

Verify Lafiya emergency-card QR codes against on-chain attestations. See
[docs/verifier-sdk.md](../../docs/verifier-sdk.md) for the full guide
(verdicts, transport errors, localized responder strings).

```ts
import { LafiyaVerifier, localizeVerdict } from "@lafiya/verifier";

const result = await verifier.verifyCard(qrPayload, { signal, timeoutMs: 5_000 });
const text = await localizeVerdict(result.verdict, navigator.language);
```

## Compatibility

Each row is exercised by `pnpm smoke` against the packed tarball
(`scripts/smoke-packed.mjs`) in CI on Node.js 20.19 and 22.12. The last column
is the smoke target id; the script fails if this table and the tested
targets drift apart.

| Runtime / toolchain | Minimum version | Entry point | Smoke target |
|---|---|---|---|
| Node.js, ES modules (`import`) | 20.19 / 22.12 | `dist/esm` | `node-esm` |
| Node.js, CommonJS (`require`) | 20.19 / 22.12 (unflagged `require(esm)`, needed for the ESM-only bindings) | `dist/cjs` | `node-cjs` |
| Bundlers (esbuild; any tool honouring `exports`) | esbuild 0.24 | `dist/esm` | `bundler` |
| Web-standard worker runtimes (edge / Workers-style, no Node built-ins) | Edge Runtime 5 | `dist/esm`, bundled | `worker` |

Node.js 21.x and 22.0–22.11 are not supported (`engines`: `^20.19.0 || >=22.12.0`).
Worker bundles must define `process.env.NODE_DEBUG` (e.g. `false`) and `global`
(as `globalThis`) for the Stellar SDK, as the `worker` smoke target does.

TypeScript consumers are checked with
[`@arethetypeswrong/cli`](https://github.com/arethetypeswrong/arethetypeswrong.github.io)
(`node10`, `node16` from CJS and ESM, `bundler`) and the manifest with
[`publint`](https://publint.dev): `pnpm check:package`.
