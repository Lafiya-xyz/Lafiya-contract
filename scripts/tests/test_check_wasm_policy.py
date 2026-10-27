"""Tests for scripts/check_wasm_policy.py.

Each negative fixture is a minimal module assembled below that breaks exactly
one rule, proving that rule fires on its own.
"""

import json
import pathlib
from typing import Any

import check_wasm_policy as policy_check
import pytest


def leb(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def vec(items: list[bytes]) -> bytes:
    return leb(len(items)) + b"".join(items)


def name(s: str) -> bytes:
    return leb(len(s)) + s.encode()


def section(section_id: int, payload: bytes) -> bytes:
    return bytes([section_id]) + leb(len(payload)) + payload


def module(
    *,
    params: bytes = b"",
    imports: list[bytes] | None = None,
    start: bool = False,
    locals_: bytes = b"\x00",
    code: bytes = b"\x0b",
    pad: int = 0,
) -> bytes:
    """One `() -> ()` function (plus optional imports/start/padding)."""
    if imports is None:
        imports = [name("l") + name("1") + b"\x00" + leb(0)]
    body = locals_ + code
    sections = [
        section(1, vec([b"\x60" + vec([params[i : i + 1] for i in range(len(params))]) + b"\x00"])),
        section(2, vec(imports)),
        section(3, vec([leb(0)])),
    ]
    if start:
        sections.append(section(8, leb(len(imports))))
    sections.append(section(10, vec([leb(len(body)) + body])))
    if pad:
        sections.append(section(0, name("pad") + bytes(pad)))
    return b"\0asm\x01\x00\x00\x00" + b"".join(sections)


FUNC_L1 = name("l") + name("1") + b"\x00" + leb(0)


@pytest.fixture
def policy(tmp_path: pathlib.Path) -> pathlib.Path:
    path = tmp_path / "policy.json"
    path.write_text(
        json.dumps(
            {
                "contracts": {
                    "demo": {
                        "wasm": "demo.wasm",
                        "max_size_bytes": 200,
                        "imports": {"l.1": "ledger::get_contract_data"},
                    }
                }
            }
        )
    )
    return path


def run(tmp_path: pathlib.Path, policy: pathlib.Path, wasm: bytes, *extra: str) -> int:
    path = tmp_path / "demo.wasm"
    path.write_bytes(wasm)
    return policy_check.main(["--policy", str(policy), *extra, str(path)])


def test_clean_module_passes(
    tmp_path: pathlib.Path, policy: pathlib.Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # i32.const 1, i64.const -1, drop, drop, block/loop/br_if, i32 load/store.
    code = (
        b"\x41\x01\x42\x7f\x1a\x1a"
        b"\x02\x40\x03\x40\x41\x00\x0d\x01\x0b\x0b"
        b"\x41\x00\x41\x00\x28\x02\x00\x36\x02\x00"
        b"\x0b"
    )
    assert run(tmp_path, policy, module(code=code, locals_=b"\x01\x01\x7e")) == 0
    assert "ok" in capsys.readouterr().out


@pytest.mark.parametrize(
    ("fixture", "message"),
    [
        (
            module(imports=[FUNC_L1, name("x") + name("7") + b"\x00" + leb(0)]),
            "unexpected host import x.7",
        ),
        (
            module(imports=[FUNC_L1, name("env") + name("memory") + b"\x02\x00\x01"]),
            "non-function import env.memory (memory)",
        ),
        (module(start=True), "start function declared"),
        (module(params=b"\x7c"), "floating-point type in function signature f64"),
        (module(locals_=b"\x01\x01\x7d"), "floating-point type in local f32"),
        (
            module(code=b"\x43\x00\x00\x80\x3f\x1a\x0b"),
            "disallowed instruction: float opcode 0x43",
        ),
        (
            module(code=b"\x41\x00\x41\x00\x41\x00\xfc\x0b\x00\x0b"),
            "disallowed instruction: misc (0xFC) opcode 11",
        ),
        (
            module(code=b"\xfd\x0c" + bytes(16) + b"\x1a\x0b"),
            "disallowed instruction: SIMD (0xFD) opcode 12",
        ),
        (module(code=b"\xd2\x00\x1a\x0b"), "disallowed instruction: unsupported opcode 0xd2"),
        (module(pad=300), "exceeds limit 200"),
    ],
)
def test_each_rule_fires(
    tmp_path: pathlib.Path,
    policy: pathlib.Path,
    capsys: pytest.CaptureFixture[str],
    fixture: bytes,
    message: str,
) -> None:
    assert run(tmp_path, policy, fixture) == 1
    out = capsys.readouterr().out
    assert message in out
    # Each fixture breaks exactly one rule.
    assert len([line for line in out.splitlines() if "note:" not in line]) == 1


def test_unused_import_is_reported_not_failed(
    tmp_path: pathlib.Path, policy: pathlib.Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert run(tmp_path, policy, module(imports=[])) == 0
    assert "allowlisted import l.1 is no longer used" in capsys.readouterr().out


def test_markdown_reports_diff(tmp_path: pathlib.Path, policy: pathlib.Path) -> None:
    env = tmp_path / "env.json"
    env.write_text(
        json.dumps(
            {"modules": [{"name": "x", "export": "x", "functions": [{"export": "7", "name": "z"}]}]}
        )
    )
    summary = tmp_path / "summary.md"
    fixture = module(imports=[name("x") + name("7") + b"\x00" + leb(0)])
    code = run(tmp_path, policy, fixture, "--markdown", str(summary), "--env-json", str(env))
    assert code == 1
    report = summary.read_text()
    assert "| `demo` |" in report
    assert "+ x.7  x::z" in report
    assert "- l.1  ledger::get_contract_data  (allowlisted, no longer imported)" in report


def test_unknown_wasm_and_garbage_fail(
    tmp_path: pathlib.Path, policy: pathlib.Path, capsys: pytest.CaptureFixture[str]
) -> None:
    other = tmp_path / "other.wasm"
    other.write_bytes(module())
    assert policy_check.main(["--policy", str(policy), str(other)]) == 1
    assert run(tmp_path, policy, b"\0asm\x01\x00\x00\x00\x01\x05\x01") == 1
    assert run(tmp_path, policy, b"junk") == 1
    out = capsys.readouterr().out
    assert "no entry in" in out
    assert "cannot parse wasm" in out


def test_update_rewrites_allowlist(tmp_path: pathlib.Path, policy: pathlib.Path) -> None:
    fixture = module(imports=[name("x") + name("7") + b"\x00" + leb(0)], pad=300)
    assert run(tmp_path, policy, fixture, "--update") == 0
    rules: dict[str, Any] = json.loads(policy.read_text())["contracts"]["demo"]
    assert rules["imports"] == {"x.7": "unknown"}
    assert rules["max_size_bytes"] == int(len(fixture) * policy_check.SIZE_HEADROOM)


def test_committed_policy_covers_every_contract() -> None:
    committed = json.loads(policy_check.DEFAULT_POLICY.read_text())["contracts"]
    contracts = {p.name for p in (policy_check.ROOT / "contracts").iterdir() if p.is_dir()}
    assert set(committed) == contracts
