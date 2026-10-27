// Smoke-tests the *packed tarball*, never the source tree: packs the SDK
// (and the bindings it depends on), installs them into a scratch consumer
// project with npm, and imports the package from each supported runtime.
// The runtimes must match the compatibility table in README.md.
//
//   pnpm build && pnpm smoke
import { execFileSync } from "node:child_process";
import { cpSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { EdgeVM } from "@edge-runtime/vm";

const pkgDir = fileURLToPath(new URL("..", import.meta.url));
const bindingsDir = join(pkgDir, "..", "..", "bindings");
const sh = (cmd, args, cwd) => execFileSync(cmd, args, { cwd, stdio: ["ignore", "pipe", "inherit"] }).toString();

/** Bundler `define`s a worker build needs (mirrored in README.md). */
const WORKER_DEFINE = { "process.env.NODE_DEBUG": "false", global: "globalThis" };

const TARGETS = {
  "node-esm": (app) => sh(process.execPath, ["esm.mjs"], app),
  "node-cjs": (app) => sh(process.execPath, ["cjs.cjs"], app),
  bundler: async (app) => {
    const outfile = join(app, "bundle.mjs");
    await build({
      entryPoints: [join(app, "bundle-entry.mjs")],
      bundle: true,
      platform: "node",
      format: "esm",
      outfile,
      logLevel: "error",
      banner: { js: "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url);" },
    });
    return sh(process.execPath, ["-e", `await import(${JSON.stringify(outfile)}); console.log(await globalThis.__smoke)`, "--input-type=module"], app);
  },
  worker: async (app) => {
    // A web-standard worker runtime (no Node built-ins), as on edge/Workers.
    // `define` is the documented bundler setup for the Stellar SDK there.
    const result = await build({
      entryPoints: [join(app, "bundle-entry.mjs")],
      bundle: true,
      platform: "browser",
      conditions: ["worker", "browser"],
      format: "iife",
      define: WORKER_DEFINE,
      write: false,
      logLevel: "error",
    });
    const vm = new EdgeVM();
    vm.evaluate("globalThis.__runtime = 'worker'");
    vm.evaluate(result.outputFiles[0].text);
    return vm.evaluate("globalThis.__smoke");
  },
};

// README compatibility table rows carry the target id in backticks in the
// last column; they must be exactly the targets exercised here.
const readme = readFileSync(join(pkgDir, "README.md"), "utf8");
const section = readme.split("## Compatibility")[1]?.split("\n## ")[0] ?? "";
const documented = [...section.matchAll(/^\|.*\|\s*`([a-z-]+)`\s*\|\s*$/gm)].map((m) => m[1]);
const missing = Object.keys(TARGETS).filter((t) => !documented.includes(t));
const extra = documented.filter((t) => !(t in TARGETS));
if (missing.length || extra.length) {
  console.error(`README compatibility table out of sync: missing ${missing}, untested ${extra}`);
  process.exit(1);
}

const work = mkdtempSync(join(tmpdir(), "lafiya-verifier-smoke-"));
try {
  sh("pnpm", ["pack", "--pack-destination", work], pkgDir);
  for (const name of ["attestation-registry", "attester-registry"]) {
    sh("npm", ["pack", "--silent", "--pack-destination", work], join(bindingsDir, name));
  }
  const tarballs = Object.fromEntries(
    readdirSync(work)
      .filter((f) => f.endsWith(".tgz"))
      .map((f) => [f.replace(/^lafiya-(.*)-\d+\.\d+\.\d+\.tgz$/, "@lafiya/$1"), `file:../${f}`]),
  );
  const app = join(work, "app");
  cpSync(join(pkgDir, "smoke"), app, { recursive: true });
  writeFileSync(
    join(app, "package.json"),
    JSON.stringify({ private: true, type: "module", dependencies: tarballs, overrides: tarballs }, null, 2),
  );
  sh("npm", ["install", "--no-audit", "--no-fund", "--loglevel=error"], app);

  let failed = 0;
  for (const [name, runTarget] of Object.entries(TARGETS)) {
    try {
      console.log(`${name}: ${String(await runTarget(app)).trim()}`);
    } catch (err) {
      failed++;
      console.error(`${name}: FAILED\n${err?.stack ?? err}`);
    }
  }
  if (failed) process.exitCode = 1;
} finally {
  rmSync(work, { recursive: true, force: true });
}
