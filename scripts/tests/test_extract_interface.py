import pathlib
from typing import Any

import pytest
from conftest import completed, load_fixture

import extract_interface

RAW = (pathlib.Path(__file__).parent / "fixtures" / "multisig_account.interface.json").read_text()


@pytest.fixture
def fake_cli(monkeypatch: pytest.MonkeyPatch) -> list[list[str]]:
    calls: list[list[str]] = []

    def run(cmd: list[str], **_: Any) -> Any:
        calls.append(cmd)
        return completed(RAW)

    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/stellar")
    monkeypatch.setattr("subprocess.run", run)
    return calls


def test_extract_decodes_fixture_spec(fake_cli: list[list[str]], wasm: pathlib.Path) -> None:
    entries = extract_interface.extract(wasm)

    assert fake_cli[0][:4] == ["stellar", "contract", "info", "interface"]
    assert [(e["kind"], e["name"]) for e in entries] == [
        ("function_v0", "__check_auth"),
        ("function_v0", "__constructor"),
        ("function_v0", "get_interface"),
        ("udt_error_enum_v0", "Error"),
        ("udt_struct_v0", "InterfaceInfo"),
        ("udt_struct_v0", "Signature"),
    ]
    assert "doc" not in str(entries)
    fn = entries[2]["spec"]
    assert fn["inputs"] == []
    assert fn["outputs"] == [{"udt": {"name": "InterfaceInfo"}}]
    fields = {f["name"]: f["type"] for f in entries[4]["spec"]["fields"]}
    assert fields["interface_version"] == "u32"
    assert fields["features"] == {"vec": {"element_type": "symbol"}}


def test_strip_docs_is_recursive() -> None:
    node = {"doc": "x", "a": [{"doc": "y", "b": 1}]}
    assert extract_interface._strip_docs(node) == {"a": [{"b": 1}]}


def test_missing_wasm(fake_cli: list[list[str]], tmp_path: pathlib.Path) -> None:
    with pytest.raises(FileNotFoundError):
        extract_interface.extract(tmp_path / "missing.wasm")


def test_missing_cli(monkeypatch: pytest.MonkeyPatch, wasm: pathlib.Path) -> None:
    monkeypatch.setattr("shutil.which", lambda _: None)
    with pytest.raises(SystemExit, match="not found"):
        extract_interface.extract(wasm)


def test_cli_failure(monkeypatch: pytest.MonkeyPatch, wasm: pathlib.Path) -> None:
    monkeypatch.setattr("shutil.which", lambda _: "stellar")
    monkeypatch.setattr("subprocess.run", lambda *a, **k: completed(returncode=1, stderr="boom"))
    with pytest.raises(SystemExit, match="boom"):
        extract_interface.extract(wasm)


def test_main(
    fake_cli: list[list[str]],
    wasm: pathlib.Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr("sys.argv", ["extract_interface.py", str(wasm)])
    extract_interface.main()
    assert '"get_interface"' in capsys.readouterr().out
    assert load_fixture("multisig_account.interface.json")


def test_main_usage(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("sys.argv", ["extract_interface.py"])
    with pytest.raises(SystemExit, match="usage"):
        extract_interface.main()
