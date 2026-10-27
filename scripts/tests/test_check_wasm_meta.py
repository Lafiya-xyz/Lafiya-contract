"""Tests for check_wasm_meta.py."""

import pathlib
import struct

import pytest

import check_wasm_meta as cwm


def xdr_str(value: str) -> bytes:
    raw = value.encode()
    return struct.pack(">I", len(raw)) + raw + b"\0" * (-len(raw) % 4)


def leb(n: int) -> bytes:
    out = bytearray()
    while True:
        byte = n & 0x7F
        n >>= 7
        out.append(byte | (0x80 if n else 0))
        if not n:
            return bytes(out)


def wasm(meta: dict[str, str] | None) -> bytes:
    module = b"\0asm\x01\0\0\0"
    if meta is not None:
        body = b"".join(struct.pack(">I", 0) + xdr_str(k) + xdr_str(v) for k, v in meta.items())
        name = b"contractmetav0"
        payload = leb(len(name)) + name + body
        module += b"\0" + leb(len(payload)) + payload
    return module


def good_meta(crate: str = "attester-registry") -> dict[str, str]:
    expected = cwm.repo_contracts()[crate]
    return {
        **{k: v for k, v in cwm.REQUIRED.items() if v},
        "crate_name": crate,
        "source_rev": "abc123",
        **expected,
    }


def run(monkeypatch: pytest.MonkeyPatch, path: pathlib.Path, *extra: str) -> int:
    monkeypatch.setattr("sys.argv", ["prog", "--commit", "abc123", *extra, str(path)])
    return cwm.main()


def test_complete_metadata_passes(tmp_path, monkeypatch):
    path = tmp_path / "ok.wasm"
    path.write_bytes(wasm(good_meta()))
    assert run(monkeypatch, path) == 0


def test_wasm_without_metadata_fails(tmp_path, monkeypatch):
    path = tmp_path / "bare.wasm"
    path.write_bytes(wasm(None))
    assert run(monkeypatch, path) == 1


def test_missing_interface_version_fails(tmp_path, monkeypatch):
    meta = good_meta()
    del meta["interface_version"]
    path = tmp_path / "partial.wasm"
    path.write_bytes(wasm(meta))
    assert run(monkeypatch, path) == 1


def test_version_disagreeing_with_cargo_fails(tmp_path, monkeypatch):
    meta = good_meta() | {"crate_version": "99.0.0"}
    path = tmp_path / "stale.wasm"
    path.write_bytes(wasm(meta))
    assert run(monkeypatch, path) == 1


def test_require_all_flags_missing_contracts(tmp_path, monkeypatch):
    path = tmp_path / "one.wasm"
    path.write_bytes(wasm(good_meta()))
    assert run(monkeypatch, path, "--require-all") == 1
