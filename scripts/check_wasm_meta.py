#!/usr/bin/env python3
"""Assert that contract wasm carries the source-provenance metadata.

Decodes every `contractmetav0` custom section (SEP-46: a stream of XDR
`SCMetaEntry` values) and checks the keys embedded by `contractmeta!` in
contracts/*/src/lib.rs. Stdlib only, so CI needs no stellar-cli.

Usage:
    scripts/check_wasm_meta.py [--commit SHA] WASM...

With --commit, `source_rev` must equal SHA (use on release builds).
Each wasm is also cross-checked against the repository: `crate_name`
must name a contract crate, and `crate_version` / `interface_version` must
equal the Cargo package version and that crate's `INTERFACE_VERSION`. With
--require-all, every contract crate must be covered by one of the WASM files.
--digest prints each wasm's metadata digest (sha256 of the sorted key=value
lines) as `<crate_name> <digest>`; the release manifest records it.
Policy: docs/stability-policy.md.
"""
import argparse
import hashlib
import re
import struct
import tomllib
import sys
from pathlib import Path

REQUIRED = {
    "source_repo": "github:Lafiya-xyz/Lafiya-contract",
    "home_domain": "lafiya-xyz.github.io",
    "crate_name": None,
    "crate_version": None,
    "source_rev": None,
    "interface_version": None,
}

ROOT = Path(__file__).resolve().parent.parent


def repo_contracts() -> dict[str, dict[str, str]]:
    """Expected metadata per contract crate, derived from Cargo and source."""
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
    version = workspace["workspace"]["package"]["version"]
    out = {}
    for crate in sorted((ROOT / "contracts").iterdir()):
        cargo = crate / "Cargo.toml"
        if not cargo.is_file():
            continue
        name = tomllib.loads(cargo.read_text())["package"]["name"]
        src = (crate / "src" / "lib.rs").read_text()
        m = re.search(r"const\s+INTERFACE_VERSION\s*:\s*u32\s*=\s*(\d+)", src)
        out[name] = {
            "crate_version": version,
            "interface_version": m.group(1) if m else "",
        }
    return out


def meta_digest(meta: dict[str, str]) -> str:
    lines = "".join(f"{k}={meta[k]}\n" for k in sorted(meta))
    return hashlib.sha256(lines.encode()).hexdigest()


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
    parser.add_argument("--require-all", action="store_true")
    parser.add_argument("--digest", action="store_true")
    parser.add_argument("wasm", nargs="+", type=Path)
    args = parser.parse_args()

    contracts = repo_contracts()
    seen: set[str] = set()
    failed = False
    for path in args.wasm:
        meta = read_meta(path.read_bytes())
        expected = dict(REQUIRED, source_rev=args.commit)
        expected.update(contracts.get(meta.get("crate_name", ""), {}))
        bad = [
            (key, want, meta.get(key))
            for key, want in expected.items()
            if not meta.get(key) or (want is not None and meta[key] != want)
        ]
        if meta.get("crate_name") not in contracts:
            bad.append(("crate_name", "a contracts/* crate", meta.get("crate_name")))
        seen.add(meta.get("crate_name", ""))
        for key, want, got in bad:
            print(f"{path}: meta {key}={got!r}, expected {want or 'non-empty'!r}")
        failed = failed or bool(bad)
        if not bad:
            print(f"{path}: " + ", ".join(f"{k}={meta[k]}" for k in REQUIRED))
            if args.digest:
                print(f"{meta['crate_name']} {meta_digest(meta)}")
    if args.require_all:
        for name in sorted(set(contracts) - seen):
            print(f"no wasm provided for contract crate {name}")
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
