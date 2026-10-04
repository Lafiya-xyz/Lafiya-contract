import fs from "fs";
import path from "path";
import { hashRecord } from "./dist/record.js";

// Load test record
const recordPath = path.resolve("../../tests/integration/test_record.json");
const content = fs.readFileSync(recordPath, "utf8");
const record = JSON.parse(content);

// Compute hash
const hashBuf = hashRecord(record);
const computedHash = hashBuf.toString("hex");

console.log("Computed TS Hash:", computedHash);

// We will replace this with the actual Rust output hash
const expectedRustHash = "40d62532537d0387e163faff4e732db5b7e5348785c6719affa7e9ffd6624d0b";

if (computedHash === expectedRustHash) {
  console.log("SUCCESS: TS and Rust hashes match perfectly!");
  process.exit(0);
} else {
  console.error("ERROR: Hash mismatch!");
  console.error("Expected:", expectedRustHash);
  console.error("Computed:", computedHash);
  process.exit(1);
}
