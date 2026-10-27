#!/usr/bin/env python3
"""Check contract wasm against the per-contract host-surface policy.

The policy (security/wasm-policy.json, CODEOWNERS-protected) lists, for every
contract, the exact Soroban host functions (`module.name` imports) the wasm may
use and its maximum size. A build fails when it:

  - imports a host function not on the contract's allowlist;
  - imports anything other than a function (memory, table, global);
  - declares a start function;
  - uses a floating-point type or instruction, or one outside the MVP
    integer subset Soroban runs (SIMD, atomics, unknown opcodes);
  - exceeds the contract's size limit.

Allowlisted imports the wasm no longer uses are reported so the list can be
tightened, but do not fail the check. Stdlib only, so CI needs no wabt.

Usage:
    scripts/check_wasm_policy.py [--policy PATH] [--env-json PATH] [--markdown PATH]
                                 [--update] WASM...

--markdown appends the per-contract diff to PATH (pass "$GITHUB_STEP_SUMMARY"
in CI). --update rewrites the allowlist and size limits from the given wasm;
review the resulting diff like any other policy change. --env-json points at
soroban-env-common's env.json so imports are labelled with host function names
(`l.1` -> `has_contract_data`).
"""

import argparse
import json
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = ROOT / "security" / "wasm-policy.json"

# Headroom added to the measured size by --update.
SIZE_HEADROOM = 1.10

IMPORT_KINDS = {0: "func", 1: "table", 2: "memory", 3: "global"}
VALTYPES = {0x7F: "i32", 0x7E: "i64", 0x7D: "f32", 0x7C: "f64", 0x7B: "v128"}
FLOAT_TYPES = {"f32", "f64", "v128"}

# Opcodes with no immediates in the MVP integer subset (plus sign-extension,
# which wasm32v1-none and the Soroban VM both enable).
PLAIN_OPS = (
    {0x00, 0x01, 0x05, 0x0B, 0x0F, 0x1A, 0x1B}
    | set(range(0x45, 0x5B))  # i32/i64 comparisons
    | set(range(0x67, 0x8B))  # i32/i64 arithmetic
    | {0xA7, 0xAC, 0xAD}  # i32.wrap_i64, i64.extend_i32_s/u
    | set(range(0xC0, 0xC5))  # sign-extension
)
FLOAT_OPS = (
    {0x2A, 0x2B, 0x38, 0x39, 0x43, 0x44}  # f32/f64 load, store, const
    | set(range(0x5B, 0x67))  # f32/f64 comparisons
    | set(range(0x8B, 0xA7))  # f32/f64 arithmetic
    | set(range(0xA8, 0xAC))  # i32.trunc_f*
    | set(range(0xAE, 0xC0))  # i64.trunc_f*, conversions, reinterprets
)
BLOCK_OPS = {0x02, 0x03, 0x04}
INDEX_OPS = {0x0C, 0x0D, 0x10, 0x20, 0x21, 0x22, 0x23, 0x24}
MEMARG_OPS = set(range(0x28, 0x3F)) - FLOAT_OPS
PREFIX_NAMES = {0xFC: "misc (0xFC)", 0xFD: "SIMD (0xFD)", 0xFE: "atomics (0xFE)"}


class WasmError(ValueError):
    pass


class Reader:
    def __init__(self, buf: bytes, pos: int = 0, end: int | None = None) -> None:
        self.buf = buf
        self.pos = pos
        self.end = len(buf) if end is None else end

    def byte(self) -> int:
        if self.pos >= self.end:
            raise WasmError("unexpected end of section")
        b = self.buf[self.pos]
        self.pos += 1
        return b

    def bytes(self, n: int) -> bytes:
        if self.pos + n > self.end:
            raise WasmError("unexpected end of section")
        out = self.buf[self.pos : self.pos + n]
        self.pos += n
        return out

    def leb(self, signed: bool = False) -> int:
        result = shift = 0
        while True:
            b = self.byte()
            result |= (b & 0x7F) << shift
            shift += 7
            if not b & 0x80:
                if signed and b & 0x40:
                    result -= 1 << shift
                return result

    def name(self) -> str:
        return self.bytes(self.leb()).decode()

    def valtype(self) -> str:
        b = self.byte()
        return VALTYPES.get(b, f"0x{b:02x}")

    def limits(self) -> None:
        flags = self.leb()
        self.leb()
        if flags & 1:
            self.leb()


@dataclass
class Module:
    size: int
    imports: list[tuple[str, str, str]] = field(default_factory=list)  # module, name, kind
    start: int | None = None
    float_types: list[str] = field(default_factory=list)
    bad_ops: dict[str, int] = field(default_factory=dict)  # description -> count


def scan_expr(r: Reader, mod: Module) -> None:
    """Walk one function body (or init expr) up to its closing `end`."""
    depth = 1
    while depth:
        op = r.byte()
        if op in PLAIN_OPS:
            if op == 0x0B:
                depth -= 1
        elif op in BLOCK_OPS:
            depth += 1
            bt = r.buf[r.pos]
            if bt == 0x40 or bt in VALTYPES:
                r.pos += 1
                if VALTYPES.get(bt) in FLOAT_TYPES:
                    mod.float_types.append(f"block type {VALTYPES[bt]}")
            else:
                r.leb(signed=True)
        elif op in INDEX_OPS:
            r.leb()
        elif op == 0x0E:  # br_table
            for _ in range(r.leb() + 1):
                r.leb()
        elif op == 0x11:  # call_indirect
            r.leb()
            r.leb()
        elif op in MEMARG_OPS:
            r.leb()
            r.leb()
        elif op in (0x3F, 0x40):  # memory.size / memory.grow
            r.byte()
        elif op == 0x41:
            r.leb(signed=True)
        elif op == 0x42:
            r.leb(signed=True)
        elif op in FLOAT_OPS:
            key = f"float opcode 0x{op:02x}"
            mod.bad_ops[key] = mod.bad_ops.get(key, 0) + 1
            if op in (0x2A, 0x2B, 0x38, 0x39):
                r.leb()
                r.leb()
            elif op == 0x43:
                r.bytes(4)
            elif op == 0x44:
                r.bytes(8)
        elif op in PREFIX_NAMES:
            sub = r.leb()
            key = f"{PREFIX_NAMES[op]} opcode {sub}"
            mod.bad_ops[key] = mod.bad_ops.get(key, 0) + 1
            # Immediates of prefixed ops vary; stop decoding this body.
            r.pos = r.end
            return
        else:
            key = f"unsupported opcode 0x{op:02x}"
            mod.bad_ops[key] = mod.bad_ops.get(key, 0) + 1
            r.pos = r.end
            return


def parse(wasm: bytes) -> Module:
    if wasm[:4] != b"\0asm":
        raise WasmError("not a wasm module")
    mod = Module(size=len(wasm))
    r = Reader(wasm, 8)
    while r.pos < len(wasm):
        section_id = r.byte()
        size = r.leb()
        s = Reader(wasm, r.pos, r.pos + size)
        if section_id == 1:  # type
            for _ in range(s.leb()):
                if s.byte() != 0x60:
                    raise WasmError("malformed function type")
                for _ in range(2):  # params, results
                    for _ in range(s.leb()):
                        t = s.valtype()
                        if t in FLOAT_TYPES:
                            mod.float_types.append(f"function signature {t}")
        elif section_id == 2:  # import
            for _ in range(s.leb()):
                module, name, kind = s.name(), s.name(), s.byte()
                mod.imports.append((module, name, IMPORT_KINDS.get(kind, f"kind {kind}")))
                if kind == 0:
                    s.leb()
                elif kind == 1:
                    s.byte()
                    s.limits()
                elif kind == 2:
                    s.limits()
                elif kind == 3:
                    s.valtype()
                    s.byte()
        elif section_id == 6:  # global
            for _ in range(s.leb()):
                t = s.valtype()
                if t in FLOAT_TYPES:
                    mod.float_types.append(f"global {t}")
                s.byte()
                scan_expr(s, mod)
        elif section_id == 8:  # start
            mod.start = s.leb()
        elif section_id == 10:  # code
            for _ in range(s.leb()):
                body_size = s.leb()
                body = Reader(wasm, s.pos, s.pos + body_size)
                for _ in range(body.leb()):
                    body.leb()
                    t = body.valtype()
                    if t in FLOAT_TYPES:
                        mod.float_types.append(f"local {t}")
                scan_expr(body, mod)
                s.pos += body_size
        r.pos += size
    return mod


def import_key(module: str, name: str) -> str:
    return f"{module}.{name}"


def host_names(env_json: Path | None) -> dict[str, str]:
    if env_json is None:
        return {}
    env = json.loads(env_json.read_text())
    return {
        import_key(m["export"], f["export"]): f"{m['name']}::{f['name']}"
        for m in env["modules"]
        for f in m["functions"]
    }


@dataclass
class Result:
    contract: str
    path: Path
    violations: list[str]
    unexpected: list[str]
    unused: list[str]


def check(contract: str, path: Path, mod: Module, rules: dict[str, Any]) -> Result:
    allowed = set(rules["imports"])
    funcs = {import_key(m, n) for m, n, kind in mod.imports if kind == "func"}
    unexpected = sorted(funcs - allowed)
    unused = sorted(allowed - funcs)
    violations = [f"unexpected host import {key}" for key in unexpected]
    violations += [
        f"non-function import {import_key(m, n)} ({kind})"
        for m, n, kind in mod.imports
        if kind != "func"
    ]
    if mod.start is not None:
        violations.append(f"start function declared (function index {mod.start})")
    violations += [f"floating-point type in {where}" for where in sorted(set(mod.float_types))]
    violations += [
        f"disallowed instruction: {op} (x{count})" for op, count in sorted(mod.bad_ops.items())
    ]
    if mod.size > rules["max_size_bytes"]:
        violations.append(f"size {mod.size} bytes exceeds limit {rules['max_size_bytes']}")
    return Result(contract, path, violations, unexpected, unused)


def contract_for(path: Path, policy: dict[str, Any]) -> str | None:
    for name, rules in policy["contracts"].items():
        if rules["wasm"] == path.name:
            return str(name)
    return None


def markdown(results: list[Result], policy: dict[str, Any], names: dict[str, str]) -> str:
    lines = ["## Wasm policy", "", "| Contract | Size | Status |", "| --- | --- | --- |"]
    for res in results:
        limit = policy["contracts"][res.contract]["max_size_bytes"]
        size = res.path.stat().st_size
        status = "❌ " + str(len(res.violations)) + " violation(s)" if res.violations else "✅"
        lines.append(f"| `{res.contract}` | {size} / {limit} | {status} |")
    for res in results:
        if not (res.violations or res.unused):
            continue
        lines += ["", f"### `{res.contract}`", "", "```diff"]
        allowed = policy["contracts"][res.contract]["imports"]
        lines += [f"+ {key}  {names.get(key, '')}".rstrip() for key in res.unexpected]
        lines += [
            f"- {key}  {allowed[key]}  (allowlisted, no longer imported)" for key in res.unused
        ]
        lines += [f"! {v}" for v in res.violations if not v.startswith("unexpected host import")]
        lines.append("```")
    return "\n".join(lines) + "\n"


def update(policy: dict[str, Any], parsed: list[tuple[str, Module]], names: dict[str, str]) -> None:
    for contract, mod in parsed:
        rules = policy["contracts"][contract]
        keys = sorted({import_key(m, n) for m, n, kind in mod.imports if kind == "func"})
        rules["imports"] = {
            key: names.get(key) or rules["imports"].get(key, "unknown") for key in keys
        }
        rules["max_size_bytes"] = max(rules["max_size_bytes"], int(mod.size * SIZE_HEADROOM))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--env-json", type=Path)
    parser.add_argument("--markdown", type=Path)
    parser.add_argument("--update", action="store_true")
    parser.add_argument("wasm", nargs="+", type=Path)
    args = parser.parse_args(argv)

    policy = json.loads(args.policy.read_text())
    names = host_names(args.env_json)
    parsed: list[tuple[str, Module]] = []
    failed = False
    for path in args.wasm:
        contract = contract_for(path, policy)
        if contract is None:
            print(f"{path}: no entry in {args.policy}; add one for every deployed contract")
            failed = True
            continue
        try:
            parsed.append((contract, parse(path.read_bytes())))
        except (WasmError, IndexError, UnicodeDecodeError) as exc:
            print(f"{path}: cannot parse wasm: {exc}")
            failed = True

    if args.update:
        update(policy, parsed, names)
        args.policy.write_text(json.dumps(policy, indent=2) + "\n")
        print(f"updated {args.policy}")
        return 1 if failed else 0

    paths = {contract: path for path in args.wasm if (contract := contract_for(path, policy))}
    results = [
        check(contract, paths[contract], mod, policy["contracts"][contract])
        for contract, mod in parsed
    ]
    for res in results:
        for v in res.violations:
            label = names.get(v.removeprefix("unexpected host import "))
            print(f"{res.path}: {v}" + (f" ({label})" if label else ""))
        for key in res.unused:
            print(f"{res.path}: note: allowlisted import {key} is no longer used")
        if not res.violations:
            print(f"{res.path}: ok ({len(policy['contracts'][res.contract]['imports'])} imports)")
        failed = failed or bool(res.violations)
    if args.markdown:
        with args.markdown.open("a") as f:
            f.write(markdown(results, policy, names))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
