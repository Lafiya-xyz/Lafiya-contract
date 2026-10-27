import * as sdk from "@lafiya/verifier";
import { run } from "./checks.mjs";

globalThis.__smoke = run(sdk, globalThis.__runtime ?? "bundler");
