#!/usr/bin/env python3
"""Generate (or check) the machine-readable error and event catalogs.

    python3 scripts/conformance/gen_catalog.py            # write catalog/ + bindings
    python3 scripts/conformance/gen_catalog.py --check     # fail on drift, don't write

Contract error codes overlap between contracts (`3` is `NoPendingTransfer`
in `attester-registry` but `AttesterNotAllowlisted` in
`attestation-registry`), so consumers must decode errors per contract. This
script emits, from one source of truth:

- `catalog/errors.json` and `catalog/events.json`;
- `bindings/<contract>/src/catalog.ts`, typed constants plus a
  `decodeContractError(contractKind, code)` helper.

Codes, names and event shapes come from the committed interface snapshots
(`snapshots/<contract>.json`), which `check_snapshot.py` pins to the
`contractspecv0` section of the built Wasm. Human-facing docs and extra
metadata come from the `///` doc comments on each `Error` variant in the
contract source, which may carry structured tags:

    /// The caller is not allowlisted.
    /// @severity user          (user | operator | bug; default: user)
    /// @retryable true         (default: false)
    /// @since 0.1.0            (default: first release that saw the code)
    /// @deprecated <reason>    (default: null)
"""
import json
import re
import sys
import tomllib

from contracts import CONTRACTS, REPO_ROOT, SNAPSHOT_DIR

CATALOG_DIR = REPO_ROOT / "catalog"
CATALOG_VERSION = 1
SEVERITIES = {"user", "operator", "bug"}
TAG_RE = re.compile(r"^@(\w+)\s*(.*)$")


def workspace_version():
    cargo = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text())
    return cargo["workspace"]["package"]["version"]


def snake(name):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def source_error_docs(crate_dir):
    """Map `Error` variant name -> (doc text, {tag: value}) from the Rust source."""
    src = (crate_dir / "src" / "lib.rs").read_text()
    block = re.search(r"#\[contracterror\].*?pub enum Error\s*\{(.*?)\n\}", src, re.DOTALL)
    if not block:
        return {}
    out, doc = {}, []
    for line in block.group(1).splitlines():
        line = line.strip()
        if line.startswith("///"):
            doc.append(line[3:].strip())
            continue
        m = re.match(r"(\w+)\s*=\s*\d+", line)
        if m:
            text, tags = [], {}
            for d in doc:
                t = TAG_RE.match(d)
                if t:
                    tags[t.group(1)] = t.group(2).strip()
                else:
                    text.append(d)
            out[m.group(1)] = (" ".join(t for t in text if t), tags)
        doc = []
    return out


def load_snapshot(name):
    return json.loads((SNAPSHOT_DIR / f"{name}.json").read_text())


def previous_since():
    path = CATALOG_DIR / "errors.json"
    if not path.exists():
        return {}
    prev = json.loads(path.read_text())
    return {(e["contract"], e["code"], e["name"]): e["since"] for e in prev["errors"]}


def build_errors(version):
    since = previous_since()
    errors = []
    for name in sorted(CONTRACTS):
        docs = source_error_docs(CONTRACTS[name]["crate_dir"])
        kind = name.removesuffix("-registry")
        for entry in load_snapshot(name):
            if entry["kind"] != "udt_error_enum_v0":
                continue
            for case in sorted(entry["spec"]["cases"], key=lambda c: c["value"]):
                doc, tags = docs.get(case["name"], ("", {}))
                severity = tags.get("severity", "user")
                if severity not in SEVERITIES:
                    sys.exit(f"[catalog] {name}::{case['name']}: bad @severity {severity!r}")
                errors.append({
                    "contract": name,
                    "code": case["value"],
                    "name": case["name"],
                    "doc": doc,
                    "severity": severity,
                    "retryable": tags.get("retryable", "false") == "true",
                    "i18n_key": f"err.{snake(kind).replace('-', '_')}.{snake(case['name'])}",
                    "since": tags.get("since")
                    or since.get((name, case["value"], case["name"]), version),
                    "deprecated": tags.get("deprecated") or None,
                })
    return {"catalog_version": CATALOG_VERSION, "errors": errors}


def build_events():
    events = []
    for name in sorted(CONTRACTS):
        for entry in load_snapshot(name):
            if entry["kind"] != "event_v0":
                continue
            spec = entry["spec"]
            events.append({
                "contract": name,
                "name": spec["name"],
                "prefix_topics": spec["prefix_topics"],
                "data_format": spec["data_format"],
                "params": [
                    {"name": p["name"], "type": p["type_"], "location": p["location"]}
                    for p in spec["params"]
                ],
            })
    return {"catalog_version": CATALOG_VERSION, "events": events}


def render_ts(errors):
    kinds = sorted(CONTRACTS)
    return (
        "// Generated by scripts/conformance/gen_catalog.py -- do not hand-edit.\n\n"
        f"export type ContractKind = {' | '.join(json.dumps(k) for k in kinds)};\n"
        'export type ErrorSeverity = "user" | "operator" | "bug";\n\n'
        "export interface ContractErrorInfo {\n"
        "  contract: ContractKind;\n"
        "  code: number;\n"
        "  name: string;\n"
        "  doc: string;\n"
        "  severity: ErrorSeverity;\n"
        "  retryable: boolean;\n"
        "  i18n_key: string;\n"
        "  since: string;\n"
        "  deprecated: string | null;\n"
        "}\n\n"
        "export const CONTRACT_ERRORS: readonly ContractErrorInfo[] = "
        f"{json.dumps(errors['errors'], indent=2)};\n\n"
        "/** Decode a contract error code; codes overlap between contracts, so the kind is required. */\n"
        "export function decodeContractError(\n"
        "  contractKind: ContractKind,\n"
        "  code: number,\n"
        "): ContractErrorInfo | undefined {\n"
        "  return CONTRACT_ERRORS.find((e) => e.contract === contractKind && e.code === code);\n"
        "}\n"
    )


def outputs():
    errors = build_errors(workspace_version())
    files = {
        CATALOG_DIR / "errors.json": json.dumps(errors, indent=2) + "\n",
        CATALOG_DIR / "events.json": json.dumps(build_events(), indent=2) + "\n",
    }
    ts = render_ts(errors)
    for cfg in CONTRACTS.values():
        files[cfg["bindings_dir"] / "src" / "catalog.ts"] = ts
    return files


def main():
    check = "--check" in sys.argv[1:]
    stale = []
    for path, content in outputs().items():
        rel = path.relative_to(REPO_ROOT)
        if check:
            if not path.exists() or path.read_text() != content:
                stale.append(rel)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
            print(f"[catalog] wrote {rel}")
    if stale:
        for rel in stale:
            print(f"[catalog] {rel} is stale -- run `make conformance-update`")
        sys.exit(1)
    if check:
        print("[catalog] OK -- catalogs match the contract specs")


if __name__ == "__main__":
    main()
