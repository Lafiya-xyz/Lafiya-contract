#!/usr/bin/env node
// Verdict performance benchmark harness (issue #763).
//
// Measures cold and warm time-to-verdict for the standard scenario under
// Playwright network throttling (2G / 3G / unthrottled) with a 4x CPU
// slowdown, against a local RPC replay server that serves recorded Soroban
// responses. Writes a JSON report consumed by `check-verdict-budget.mjs`.
//
// Usage:
//   node scripts/verdict-bench.mjs [--runs 20] [--out verdict-bench.json]
//     [--profiles 2g,3g,unthrottled] [--fixtures fixtures/verdict-bench]
//
// The harness is intentionally dependency-light: it drives the built SDK
// bundle in a headless Chromium page and only needs `playwright` (dev
// dependency) plus Node's http server for the replay endpoint.

import { createServer } from "node:http";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pkgRoot = resolve(here, "..");

// Playwright network emulation profiles. Values mirror the DevTools presets
// so results are comparable with browser devtools measurements.
export const PROFILES = {
  "2g": { downloadThroughput: (250 * 1024) / 8, uploadThroughput: (50 * 1024) / 8, latency: 300 },
  "3g": { downloadThroughput: (1.6 * 1024 * 1024) / 8, uploadThroughput: (750 * 1024) / 8, latency: 150 },
  unthrottled: { downloadThroughput: -1, uploadThroughput: -1, latency: 0 },
};

const CPU_SLOWDOWN = 4;

function parseArgs(argv) {
  const args = {
    runs: 20,
    out: join(pkgRoot, "verdict-bench.json"),
    profiles: Object.keys(PROFILES),
    fixtures: join(pkgRoot, "fixtures", "verdict-bench"),
  };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (flag === "--runs") args.runs = Number(value);
    else if (flag === "--out") args.out = value;
    else if (flag === "--profiles") args.profiles = value.split(",").map((p) => p.trim());
    else if (flag === "--fixtures") args.fixtures = value;
    else throw new Error(`unknown argument: ${flag}`);
    i += 1;
  }
  for (const profile of args.profiles) {
    if (!PROFILES[profile]) throw new Error(`unknown profile: ${profile}`);
  }
  return args;
}

/** Nearest-rank percentile over a sorted copy of `values`. */
export function percentile(values, p) {
  if (values.length === 0) return NaN;
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.ceil((p / 100) * sorted.length);
  return sorted[Math.min(sorted.length - 1, Math.max(0, rank - 1))];
}

/**
 * Local RPC replay server. Serves the recorded JSON-RPC responses keyed by
 * request method+params hash, so the benchmark exercises the real SDK network
 * path without touching a live network.
 */
export function startReplayServer(fixturesDir) {
  const index = JSON.parse(readFileSync(join(fixturesDir, "rpc-index.json"), "utf8"));
  const benchPage = readFileSync(join(fixturesDir, "bench.html"), "utf8");
  const server = createServer((req, res) => {
    if (req.url?.startsWith("/bench.html")) {
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      res.end(benchPage);
      return;
    }
    let body = "";
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      let key = "default";
      try {
        const parsed = JSON.parse(body);
        const call = Array.isArray(parsed) ? parsed[0] : parsed;
        key = `${call?.method}:${JSON.stringify(call?.params ?? [])}`;
      } catch {
        // fall through to the default fixture
      }
      const fixture = index[key] ?? index.default;
      if (!fixture) {
        res.writeHead(404, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "no fixture" }));
        return;
      }
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify(fixture));
    });
  });
  return new Promise((resolvePromise) => {
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      resolvePromise({ server, url: `http://127.0.0.1:${port}` });
    });
  });
}

async function measureProfile({ browser, profile, runs, rpcUrl, pageUrl }) {
  const context = await browser.newContext();
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  await cdp.send("Network.enable");
  await cdp.send("Network.emulateNetworkConditions", { offline: false, ...PROFILES[profile] });
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: CPU_SLOWDOWN });

  const cold = [];
  const warm = [];
  for (let i = 0; i < runs; i += 1) {
    // Cold: fresh context per run so caches, module graph and RPC responses
    // are all discarded.
    const coldContext = await browser.newContext();
    const coldPage = await coldContext.newPage();
    const coldCdp = await coldContext.newCDPSession(coldPage);
    await coldCdp.send("Network.enable");
    await coldCdp.send("Network.emulateNetworkConditions", { offline: false, ...PROFILES[profile] });
    await coldCdp.send("Emulation.setCPUThrottlingRate", { rate: CPU_SLOWDOWN });
    cold.push(await runScenario(coldPage, rpcUrl, pageUrl));
    await coldContext.close();

    // Warm: reuse the page so the SDK bundle and RPC responses are cached.
    warm.push(await runScenario(page, rpcUrl, pageUrl));
  }
  await context.close();
  return {
    cold: { p50: percentile(cold, 50), p95: percentile(cold, 95), samples: cold },
    warm: { p50: percentile(warm, 50), p95: percentile(warm, 95), samples: warm },
  };
}

async function runScenario(page, rpcUrl, pageUrl) {
  await page.goto(`${pageUrl}?rpc=${encodeURIComponent(rpcUrl)}`, { waitUntil: "load" });
  const ms = await page.evaluate(async () => {
    const started = performance.now();
    await window.__lafiyaBench.verifyStandardScenario();
    return performance.now() - started;
  });
  return ms;
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    console.error(`verdict-bench: ${err.message}`);
    process.exit(2);
  }
  if (!existsSync(join(args.fixtures, "rpc-index.json"))) {
    console.error(`verdict-bench: missing fixtures at ${args.fixtures}`);
    process.exit(2);
  }
  const { chromium } = await import("playwright");
  const { server, url: rpcUrl } = await startReplayServer(args.fixtures);
  const browser = await chromium.launch();
  const pageUrl = `${rpcUrl}/bench.html`;
  const report = { generatedAt: new Date().toISOString(), cpuSlowdown: CPU_SLOWDOWN, runs: args.runs, profiles: {} };
  try {
    for (const profile of args.profiles) {
      report.profiles[profile] = await measureProfile({ browser, profile, runs: args.runs, rpcUrl, pageUrl });
    }
  } finally {
    await browser.close();
    server.close();
  }
  writeFileSync(args.out, `${JSON.stringify(report, null, 2)}\n`);
  for (const [profile, phases] of Object.entries(report.profiles)) {
    console.log(`${profile}: cold p95 ${Math.round(phases.cold.p95)}ms, warm p95 ${Math.round(phases.warm.p95)}ms`);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main().catch((err) => {
    console.error(`verdict-bench: ${err.stack ?? err}`);
    process.exit(1);
  });
}
