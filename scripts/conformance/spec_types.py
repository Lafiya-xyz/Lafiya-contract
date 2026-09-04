"""Render a `stellar contract info interface --output json` type node as a
short human-readable string, e.g. {"option": {"value_type": "address"}} ->
"Option<Address>".
"""

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


def render_type(t):
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
        return body["name"]
    return json_fallback(t)


def json_fallback(t):
    import json

    return json.dumps(t)
