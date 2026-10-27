const sdk = require("@lafiya/verifier");

if (!require.resolve("@lafiya/verifier").includes("/dist/cjs/")) {
  throw new Error("[smoke:node-cjs] require() did not resolve the CommonJS build");
}
import("./checks.mjs")
  .then(({ run }) => run(sdk, "node-cjs"))
  .then(console.log, (err) => {
    console.error(err);
    process.exit(1);
  });
