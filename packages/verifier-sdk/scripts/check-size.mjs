// Size budget for the SDK's own code (dependencies are external and gated by
// check-supply-chain.mjs). For every entry point in package.json `exports`,
// sums the gzipped size of the entry file and every module it reaches
// through relative imports, and fails CI if any exceeds its budget.
//
//   node scripts/check-size.mjs [--markdown report.md]
//
// The markdown report is also appended to $GITHUB_STEP_SUMMARY when set; the
// verifier-sdk workflow posts it on pull requests.
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";

// Gzipped bytes per entry point and module format. Raise deliberately, in
// the same PR as the growth, with the reason in the PR description.
const BUDGETS = {
  ".": { import: 8 * 1024, require: 9 * 1024 },
};

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
