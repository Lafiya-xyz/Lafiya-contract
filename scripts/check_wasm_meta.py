#!/usr/bin/env python3
"""Assert that contract wasm carries the source-provenance metadata.

Decodes every `contractmetav0` custom section (SEP-46: a stream of XDR
`SCMetaEntry` values) and checks the keys embedded by `contractmeta!` in
contracts/*/src/lib.rs. Stdlib only, so CI needs no stellar-cli.

Usage:
    scripts/check_wasm_meta.py [--commit SHA] WASM...

With --commit, `source_rev` must equal SHA (use on release builds).
"""
import argparse
import struct
import sys
from pathlib import Path

REQUIRED = {
    "source_repo": "github:Lafiya-xyz/Lafiya-contract",
    "home_domain": "lafiya-xyz.github.io",
    "crate_name": None,
    "crate_version": None,
    "source_rev": None,
}


def leb128(buf: bytes, pos: int) -> tuple[int, int]:
    result = shift = 0
    while True:
        byte = buf[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, pos
        shift += 7


def xdr_string(buf: bytes, pos: int) -> tuple[str, int]:
    (length,) = struct.unpack_from(">I", buf, pos)
    pos += 4
    value = buf[pos : pos + length].decode()
    return value, pos + length + (-length % 4)


def read_meta(wasm: bytes) -> dict[str, str]:
    if wasm[:4] != b"\0asm":
        raise ValueError("not a wasm module")
    meta, pos = {}, 8
    while pos < len(wasm):
        section_id = wasm[pos]
        size, pos = leb128(wasm, pos + 1)
        end = pos + size
        if section_id == 0:
            name_len, name_pos = leb128(wasm, pos)
            name = wasm[name_pos : name_pos + name_len]
            if name == b"contractmetav0":
                p = name_pos + name_len
                while p < end:
                    (kind,) = struct.unpack_from(">I", wasm, p)
                    if kind != 0:  # SC_META_V0
                        raise ValueError(f"unknown SCMetaEntry kind {kind}")
                    key, p = xdr_string(wasm, p + 4)
                    val, p = xdr_string(wasm, p)
                    meta[key] = val
        pos = end
    return meta


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--commit")
    parser.add_argument("wasm", nargs="+", type=Path)
    args = parser.parse_args()

    failed = False
    for path in args.wasm:
        meta = read_meta(path.read_bytes())
        expected = dict(REQUIRED, source_rev=args.commit)
        bad = [
            (key, want, meta.get(key))
            for key, want in expected.items()
            if not meta.get(key) or (want is not None and meta[key] != want)
        ]
        for key, want, got in bad:
            print(f"{path}: meta {key}={got!r}, expected {want or 'non-empty'!r}")
        failed = failed or bool(bad)
        if not bad:
            print(f"{path}: " + ", ".join(f"{k}={meta[k]}" for k in REQUIRED))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
