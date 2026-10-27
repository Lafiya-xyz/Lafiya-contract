// Differential-testing adapter: reads one verification-model input per line
// (JSON) on stdin and prints `{"verdict","reason","qualifier"}` per line.
// Runs against the built package (pnpm build).
import { createInterface } from "node:readline";
import { evaluateModel } from "../dist/esm/model.js";

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  if (line.trim()) process.stdout.write(JSON.stringify(evaluateModel(JSON.parse(line))) + "\n");
}
