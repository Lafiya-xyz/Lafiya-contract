// Size budget for the SDK's own code (dependencies are external). Fails CI
// if the gzipped ESM output grows past BUDGET_BYTES.
import { readdirSync, readFileSync } from "node:fs";
import { gzipSync } from "node:zlib";

const BUDGET_BYTES = 8 * 1024;
const dir = new URL("../dist/esm/", import.meta.url);
const files = readdirSync(dir).filter((f) => f.endsWith(".js"));
const total = files.reduce((sum, f) => sum + gzipSync(readFileSync(new URL(f, dir))).length, 0);

console.log(`@lafiya/verifier: ${total} bytes gzipped (budget ${BUDGET_BYTES})`);
if (total > BUDGET_BYTES) {
  console.error("size budget exceeded");
  process.exit(1);
}
