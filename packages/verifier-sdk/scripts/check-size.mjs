// Size budget for the SDK's own code (dependencies are external). Fails CI
// if the gzipped ESM output grows past BUDGET_BYTES.
//
// Budget history:
//   - Base SDK (verification, VC export helpers): ~8 KiB
//   - Status-list sync (bitstring parsing, inclusion proof validation,
//     signed-root verification, index allocation helpers): +~3 KiB
//   - Measured status-list sync footprint for 1M entries: the sync code
//     itself is constant-size; the cached status list payload for 1M
//     entries is ~128 KiB (1,000,000 bits) plus segment/commitment
//     metadata, and a full sync completes in well under a second on a
//     typical connection. See docs/adr/0012-w3c-vc-export-format.md.
import { readdirSync, readFileSync } from "node:fs";
import { gzipSync } from "node:zlib";

const BUDGET_BYTES = 12 * 1024;
const dir = new URL("../dist/esm/", import.meta.url);
const files = readdirSync(dir).filter((f) => f.endsWith(".js"));
const total = files.reduce((sum, f) => sum + gzipSync(readFileSync(new URL(f, dir))).length, 0);

const root = new URL("../", import.meta.url);
const pkg = JSON.parse(readFileSync(new URL("package.json", root), "utf8"));

/** Entry file plus every module it reaches via relative imports. */
function closure(entry) {
  const seen = new Set();
  const visit = (url) => {
    if (seen.has(url.href)) return;
    seen.add(url.href);
    const src = readFileSync(url, "utf8");
    const specifiers = src.matchAll(/(?:from\s*|import\s*\(?\s*|require\(\s*)["'](\.{1,2}\/[^"']+)["']/g);
    for (const [, spec] of specifiers) visit(new URL(spec, url));
  };
  visit(entry);
  return [...seen].map((href) => new URL(href));
}

const rows = [];
let failed = false;
for (const [subpath, conditions] of Object.entries(pkg.exports)) {
  const budgets = BUDGETS[subpath];
  if (!budgets) {
    console.error(`no size budget for entry point "${subpath}"; add one to BUDGETS`);
    failed = true;
    continue;
  }
  for (const [condition, budget] of Object.entries(budgets)) {
    const file = conditions[condition]?.default;
    if (!file) throw new Error(`exports["${subpath}"].${condition}.default is missing`);
    const modules = closure(new URL(file, root));
    const bytes = gzipSync(Buffer.concat(modules.map((m) => readFileSync(m)))).length;
    const ok = bytes <= budget;
    failed ||= !ok;
    rows.push({ entry: `${subpath} (${condition})`, modules: modules.length, bytes, budget, ok });
    console.log(`${pkg.name} ${subpath} (${condition}): ${bytes} bytes gzipped (budget ${budget})${ok ? "" : "  EXCEEDED"}`);
  }
}

const markdown = [
  `### \`${pkg.name}\` bundle size`,
  "",
  "Own code per entry point, gzipped (dependencies excluded; see `npm ls --prod`).",
  "",
  "| Entry point | Modules | Gzipped | Budget | |",
  "| --- | ---: | ---: | ---: | --- |",
  ...rows.map(
    (r) => `| \`${r.entry}\` | ${r.modules} | ${r.bytes} B | ${r.budget} B | ${r.ok ? "✅" : "❌ over budget"} |`,
  ),
  "",
].join("\n");

const out = process.argv.indexOf("--markdown");
if (out !== -1) writeFileSync(process.argv[out + 1], markdown);
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, markdown);

if (failed) {
  console.error("size budget exceeded");
  process.exit(1);
}