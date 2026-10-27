import * as sdk from "@lafiya/verifier";
import { run } from "./checks.mjs";

console.log(await run(sdk, "node-esm"));
