// Supply-chain gate for @lafiya/verifier, which runs in safety-critical UIs:
//
// - runtime dependencies must be on the allowlist below, with a reason;
// - neither this package nor its dependencies may run install scripts
//   (.npmrc `ignore-scripts=true`, pnpm `strictDepBuilds`);
// - every registry package in the lockfile must carry an sha512 integrity
//   hash, and nothing may resolve from a tarball URL or git;
// - publishing must use npm provenance.
//
//   node scripts/check-supply-chain.mjs
import { readFileSync } from "node:fs";

const root = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, root), "utf8");
const pkg = JSON.parse(read("package.json"));
const errors = [];

// Adding a runtime dependency means adding it here, in review.
const ALLOWED_DEPENDENCIES = {
  // Generated contract clients: the single source of truth for the
  // registries' interfaces. They bring @stellar/stellar-sdk (XDR, RPC).
  "@lafiya/attestation-registry": "file:../../bindings/attestation-registry",
  "@lafiya/attester-registry": "file:../../bindings/attester-registry",
};
for (const [name, spec] of Object.entries(pkg.dependencies ?? {})) {
  if (ALLOWED_DEPENDENCIES[name] !== spec) {
    errors.push(`runtime dependency ${name}@${spec} is not on the allowlist in scripts/check-supply-chain.mjs`);
  }
}
for (const field of ["optionalDependencies", "peerDependencies", "bundleDependencies", "bundledDependencies"]) {
  if (pkg[field] && Object.keys(pkg[field]).length) errors.push(`package.json must not declare ${field}`);
}

for (const hook of ["preinstall", "install", "postinstall", "prepare", "prepublish"]) {
  if (pkg.scripts?.[hook]) errors.push(`package.json must not define a "${hook}" script`);
}
if (!/^ignore-scripts=true$/m.test(read(".npmrc"))) errors.push(".npmrc must set ignore-scripts=true");
if (!/^strictDepBuilds: true$/m.test(read("pnpm-workspace.yaml"))) {
  errors.push("pnpm-workspace.yaml must set strictDepBuilds: true");
}
if (/^\s+\S+: true$/m.test(read("pnpm-workspace.yaml").split("strictDepBuilds")[0])) {
  errors.push("pnpm-workspace.yaml allowBuilds must not allow any dependency build script");
}

if (pkg.publishConfig?.provenance !== true) errors.push("publishConfig.provenance must be true");

const lock = read("pnpm-lock.yaml");
let resolutions = 0;
for (const [, body] of lock.matchAll(/^\s+resolution: \{(.*)\}$/gm)) {
  resolutions++;
  if (/^integrity: sha512-[A-Za-z0-9+/]+=*$/.test(body)) continue;
  if (/^directory: \.\.\/\.\.\/bindings\/[a-z-]+, type: directory$/.test(body)) continue;
  errors.push(`lockfile resolution without sha512 integrity: {${body}}`);
}
if (resolutions === 0) errors.push("no resolutions found in pnpm-lock.yaml");
if (/tarball:|git\+|codeload\.github\.com/.test(lock)) {
  errors.push("lockfile resolves a package from a tarball URL or git");
}

if (errors.length) {
  for (const e of errors) console.error(`supply-chain: ${e}`);
  process.exit(1);
}
console.log(`supply-chain: ok (${resolutions} lockfile resolutions checked)`);
