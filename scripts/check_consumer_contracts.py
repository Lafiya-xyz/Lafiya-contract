#!/usr/bin/env python3
"""Provider verification: every consumer expectation must hold in the indexer OpenAPI spec.

Usage: check_consumer_contracts.py [openapi.yaml] [consumer-contracts-dir]
Requires PyYAML. Exits non-zero listing each broken expectation.
"""
import json
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
INDEXER = ROOT / "crates" / "lafiya-indexer"


def resolve(spec, node):
    while isinstance(node, dict) and "$ref" in node:
        cur = spec
        for part in node["$ref"].lstrip("#/").split("/"):
            cur = cur[part]
        node = cur
    return node


def check(spec, contract):
    errors = []
    for it in contract["interactions"]:
        tag = f'{contract["consumer"]}: {it["description"]}'
        op = spec.get("paths", {}).get(it["path"], {}).get("get")
        if not op:
            errors.append(f'{tag}: GET {it["path"]} missing')
            continue
        params = {resolve(spec, p).get("name") for p in op.get("parameters", [])}
        for q in it["query"]:
            if q not in params:
                errors.append(f"{tag}: query parameter '{q}' missing")
        resp = op.get("responses", {}).get(it["status"])
        if not resp:
            errors.append(f'{tag}: response {it["status"]} missing')
            continue
        schema = resolve(spec, resp).get("content", {}).get("application/json", {}).get("schema", {})
        props = resolve(spec, schema).get("properties", {})
        required = set(resolve(spec, schema).get("required", []))
        for f in it["requiredResponseFields"]:
            if f not in props:
                errors.append(f"{tag}: response field '{f}' missing")
            elif f not in required:
                errors.append(f"{tag}: response field '{f}' no longer required")
        pg = it.get("pagination")
        if pg and pg["nullableAtEnd"] and not props.get(pg["cursorField"], {}).get("nullable"):
            errors.append(f'{tag}: {pg["cursorField"]} must stay nullable')
        if it.get("itemSchema"):
            item = resolve(spec, spec["components"]["schemas"].get(it["itemSchema"], {}))
            for f in it["requiredItemFields"]:
                if f not in item.get("properties", {}):
                    errors.append(f'{tag}: {it["itemSchema"]}.{f} missing')
    return errors


def main():
    spec_path = Path(sys.argv[1]) if len(sys.argv) > 1 else INDEXER / "openapi.yaml"
    dir_path = Path(sys.argv[2]) if len(sys.argv) > 2 else INDEXER / "consumer-contracts"
    spec = yaml.safe_load(spec_path.read_text())
    errors = []
    for f in sorted(dir_path.glob("*.json")):
        errors += check(spec, json.loads(f.read_text()))
    for e in errors:
        print("BREAKING:", e)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
