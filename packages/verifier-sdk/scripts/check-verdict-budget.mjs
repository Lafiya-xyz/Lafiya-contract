#!/usr/bin/env node
// Verdict performance budget gate (issue #763).
//
// Reads the Playwright benchmark report produced by
// `scripts/verdict-bench.mjs` (JSON: { profiles: { <name>: { cold: {p95}, warm: {p95} } } })
// and fails when a measured p95 exceeds the budget for that profile.
//
// Usage:
//   node scripts/check-verdict-budget.mjs --report verdict-bench.json \
//     [--budget verdict-budget.json] [--markdown out.md]
//
// Exit codes: 0 within budget, 1 regression, 2 usage/IO error.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const DEFAULT_BUDGET = join(here, "..", "verdict-budget.json");

function parseArgs(argv) {
  const args = { budget: DEFAULT_BUDGET, report: null, markdown: null };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (flag === "--report") args.report = value;
    else if (flag === "--budget") args.budget = value;
    else if (flag === "--markdown") args.markdown = value;
    else throw new Error(`unknown argument: ${flag}`);
    i += 1;
  }
  if (!args.report) throw new Error("--report is required");
  return args;
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

/**
 * Compare a benchmark report against a budget.
 * @returns {{ ok: boolean, rows: Array<{profile: string, phase: string, p95: number, budget: number, ok: boolean}> }}
 */
export function checkBudget(report, budget) {
  const rows = [];
  for (const [profile, limits] of Object.entries(budget.profiles ?? {})) {
    const measured = report.profiles?.[profile];
    for (const phase of ["cold", "warm"]) {
      const limit = limits[phase];
      if (typeof limit !== "number") continue;
      const p95 = measured?.[phase]?.p95;
      // A missing measurement is a regression: the gate must not pass silently.
      const ok = typeof p95 === "number" && p95 <= limit;
      rows.push({ profile, phase, p95: typeof p95 === "number" ? p95 : NaN, budget: limit, ok });
    }
  }
  return { ok: rows.every((r) => r.ok), rows };
}

function toMarkdown(result) {
  const lines = ["### Verdict performance budget", "", "| Profile | Phase | p95 (ms) | Budget (ms) | |", "|---|---|---|---|---|"];
  for (const r of result.rows) {
    const p95 = Number.isNaN(r.p95) ? "missing" : Math.round(r.p95).toString();
    lines.push(`| ${r.profile} | ${r.phase} | ${p95} | ${r.budget} | ${r.ok ? "ok" : "**over budget**"} |`);
  }
  lines.push("", result.ok ? "Within budget." : "Regression: at least one profile exceeds its budget.");
  return `${lines.join("\n")}\n`;
}

function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    console.error(`check-verdict-budget: ${err.message}`);
    process.exit(2);
  }
  let report;
  let budget;
  try {
    report = readJson(args.report);
    budget = readJson(args.budget);
  } catch (err) {
    console.error(`check-verdict-budget: cannot read input: ${err.message}`);
    process.exit(2);
  }
  const result = checkBudget(report, budget);
  const markdown = toMarkdown(result);
  process.stdout.write(markdown);
  if (args.markdown) writeFileSync(args.markdown, markdown);
  process.exit(result.ok ? 0 : 1);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
