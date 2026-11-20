"""Render a `stellar contract info interface --output json` type node as a
short human-readable string, e.g. {"option": {"value_type": "address"}} ->
"Option<Address>".

Also provides helpers to enumerate every `#[contracttype]` referenced by the
conformance specs so the Rust round-trip generator (#727) can cover new types
automatically.
"""

import json
from typing import Any, Dict, Iterable, List, Set

_SCALARS = {
    "address": "Address",
    "bool": "bool",
    "bytes": "Bytes",
    "duration": "Duration",
    "i128": "i128",
    "i256": "i256",
    "i32": "i32",
    "i64": "i64",
    "string": "String",
    "symbol": "Symbol",
    "timepoint": "Timepoint",
    "u128": "u128",
    "u256": "u256",
    "u32": "u32",
    "u64": "u64",
    "val": "Val",
    "void": "()",
}


def render_type(t: Any) -> str:
    if isinstance(t, str):
        return _SCALARS.get(t, t)
    ((kind, body),) = t.items()
    if kind == "option":
        return f"Option<{render_type(body['value_type'])}>"
    if kind == "vec":
        return f"Vec<{render_type(body['element_type'])}>"
    if kind == "map":
        return f"Map<{render_type(body['key_type'])}, {render_type(body['value_type'])}>"
    if kind == "bytes_n":
        return f"BytesN<{body['n']}>"
    if kind == "result":
        return f"Result<{render_type(body['ok_type'])}, {render_type(body['error_type'])}>"
    if kind == "tuple":
        return "(" + ", ".join(render_type(v) for v in body["value_types"]) + ")"
    if kind == "udt":
        return str(body["name"])
    return json_fallback(t)


def json_fallback(t: Any) -> str:
    return json.dumps(t)


def _iter_udt_names(t: Any) -> Iterable[str]:
    """Yield every UDT name referenced (recursively) by a type node."""
    if isinstance(t, str):
        return
    ((kind, body),) = t.items()
    if kind == "udt":
        yield str(body["name"])
    elif kind == "option":
        yield from _iter_udt_names(body["value_type"])
    elif kind == "vec":
        yield from _iter_udt_names(body["element_type"])
    elif kind == "map":
        yield from _iter_udt_names(body["key_type"])
        yield from _iter_udt_names(body["value_type"])
    elif kind == "result":
        yield from _iter_udt_names(body["ok_type"])
        yield from _iter_udt_names(body["error_type"])
    elif kind == "tuple":
        for v in body["value_types"]:
            yield from _iter_udt_names(v)


def _iter_udt_names_from_udt(udt: Dict[str, Any]) -> Iterable[str]:
    """Yield UDT names referenced by a UDT definition's fields/cases."""
    kind = udt.get("type")
    if kind == "struct":
        for case in udt.get("cases", []):
            for field in case.get("fields", []):
                yield from _iter_udt_names(field["type"])
    elif kind == "union":
        for case in udt.get("cases", []):
            for field in case.get("fields", []):
                yield from _iter_udt_names(field["type"])
    elif kind == "enum":
        for case in udt.get("cases", []):
            for field in case.get("fields", []):
                yield from _iter_udt_names(field["type"])
    elif kind == "error_enum":
        pass


def collect_contracttypes(spec: Dict[str, Any]) -> List[str]:
    """Return the sorted, de-duplicated list of every UDT name referenced by a
    single contract spec (functions, events, and nested UDT fields)."""
    names: Set[str] = set()

    for fn in spec.get("functions", []):
        for inp in fn.get("inputs", []):
            names.update(_iter_udt_names(inp["type"]))
        for out in fn.get("outputs", []):
            names.update(_iter_udt_names(out["type"]))

    for ev in spec.get("events", []):
        for field in ev.get("fields", []):
            names.update(_iter_udt_names(field["type"]))

    udts = spec.get("udt_structs", {}) or {}
    for udt in udts.values():
        names.update(_iter_udt_names_from_udt(udt))

    # UDTs may reference each other; iterate to a fixed point.
    changed = True
    while changed:
        changed = False
        for name in list(names):
            udt = udts.get(name)
            if udt is None:
                continue
            for ref in _iter_udt_names_from_udt(udt):
                if ref not in names:
                    names.add(ref)
                    changed = True

    return sorted(names)


def collect_all_contracttypes(specs: Dict[str, Dict[str, Any]]) -> Dict[str, List[str]]:
    """Given a mapping of contract name -> spec, return contract name -> sorted
    list of every `#[contracttype]` referenced by that contract."""
    return {name: collect_contracttypes(spec) for name, spec in specs.items()}


def render_contracttype_list(specs: Dict[str, Dict[str, Any]]) -> str:
    """Render the generated contracttype list as a stable JSON document that the
    Rust generator consumes and the drift test compares against."""
    return json.dumps(collect_all_contracttypes(specs), indent=2, sort_keys=True) + "\n"