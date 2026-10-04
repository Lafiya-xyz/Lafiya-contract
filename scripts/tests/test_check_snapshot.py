import copy
import json
import pathlib
from typing import Any

import pytest

import check_snapshot
from contracts import CONTRACTS

Entry = dict[str, Any]


def fn(name: str, inputs: list[Entry], output: Any = "void") -> Entry:
    return {
        "kind": "function_v0",
        "name": name,
        "spec": {"name": name, "inputs": inputs, "outputs": [output]},
    }


def error_enum(*cases: tuple[str, int]) -> Entry:
    spec = {"name": "Error", "cases": [{"name": n, "value": v} for n, v in cases]}
    return {"kind": "udt_error_enum_v0", "name": "Error", "spec": spec}


def enum(*cases: tuple[str, int]) -> Entry:
    spec = {"name": "Role", "cases": [{"name": n, "value": v} for n, v in cases]}
    return {"kind": "udt_enum_v0", "name": "Role", "spec": spec}


def union(*names: str) -> Entry:
    spec = {"name": "Key", "cases": [{"tuple_v0": {"name": n}} for n in names]}
    return {"kind": "udt_union_v0", "name": "Key", "spec": spec}


ADDR = {"name": "attester", "type_": "address"}
BASE = [
    fn("is_attester", [ADDR], "bool"),
    fn("pause", []),
    error_enum(("NotInitialized", 1), ("AlreadyInitialized", 2)),
    enum(("Admin", 0), ("Attester", 1)),
    union("Admin", "Attester"),
]


def replace(entries: list[Entry], new: Entry) -> list[Entry]:
    out = copy.deepcopy(entries)
    idx = next(i for i, e in enumerate(out) if (e["kind"], e["name"]) == (new["kind"], new["name"]))
    out[idx] = new
    return out


def test_no_change_is_not_breaking() -> None:
    assert check_snapshot.diff_entries(BASE, BASE) == ([], [], [])
    assert not check_snapshot.is_breaking(BASE, copy.deepcopy(BASE))


def test_removed_function_is_breaking() -> None:
    new = [e for e in BASE if e["name"] != "pause"]
    removed, added, changed = check_snapshot.diff_entries(BASE, new)
    assert removed == [("function_v0", "pause")] and not added and not changed
    assert check_snapshot.is_breaking(BASE, new)


def test_new_function_is_additive() -> None:
    new = [*BASE, fn("get_interface", [], {"udt": {"name": "InterfaceInfo"}})]
    assert check_snapshot.diff_entries(BASE, new)[1] == [("function_v0", "get_interface")]
    assert not check_snapshot.is_breaking(BASE, new)


def test_new_optional_argument_is_breaking() -> None:
    region = {"name": "region", "type_": {"option": {"value_type": "symbol"}}}
    new = replace(BASE, fn("is_attester", [ADDR, region], "bool"))
    assert check_snapshot.diff_entries(BASE, new)[2] == [("function_v0", "is_attester")]
    assert check_snapshot.is_breaking(BASE, new)


def test_changed_return_type_is_breaking() -> None:
    new = replace(BASE, fn("is_attester", [ADDR], "u32"))
    assert check_snapshot.is_breaking(BASE, new)


def test_new_error_variant_is_additive() -> None:
    new = replace(BASE, error_enum(("NotInitialized", 1), ("AlreadyInitialized", 2), ("New", 3)))
    assert check_snapshot.diff_entries(BASE, new)[2] == [("udt_error_enum_v0", "Error")]
    assert not check_snapshot.is_breaking(BASE, new)


def test_renumbered_error_variant_is_breaking() -> None:
    new = replace(BASE, error_enum(("NotInitialized", 1), ("AlreadyInitialized", 3)))
    assert check_snapshot.is_breaking(BASE, new)


def test_reordered_enum_is_breaking() -> None:
    new = replace(BASE, enum(("Attester", 0), ("Admin", 1)))
    assert check_snapshot.is_breaking(BASE, new)


def test_reordered_union_is_breaking() -> None:
    new = replace(BASE, union("Attester", "Admin"))
    assert check_snapshot.is_breaking(BASE, new)


# --- check / update flows -------------------------------------------------


@pytest.fixture
def env(tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch) -> dict[str, Any]:
    crate = tmp_path / "crate"
    (crate / "src").mkdir(parents=True)
    state: dict[str, Any] = {"crate": crate, "current": copy.deepcopy(BASE)}
    set_version(state, 1)
    monkeypatch.setattr(check_snapshot, "SNAPSHOT_DIR", tmp_path / "snapshots")
    monkeypatch.setattr(
        check_snapshot, "VERSIONS_FILE", tmp_path / "snapshots" / "interface_versions.json"
    )
    monkeypatch.setattr(
        check_snapshot,
        "CONTRACTS",
        {"demo": {"crate_dir": crate, "wasm_path": tmp_path / "demo.wasm"}},
    )
    monkeypatch.setattr(check_snapshot, "extract", lambda _: state["current"])
    return state


def set_version(state: dict[str, Any], version: int) -> None:
    (state["crate"] / "src" / "lib.rs").write_text(
        f"pub const INTERFACE_VERSION: u32 = {version};\n"
    )


def run(args: list[str]) -> int:
    with pytest.raises(SystemExit) as exc:
        check_snapshot.main(args)
    return int(exc.value.code or 0)


def test_check_without_snapshot_fails(env: dict[str, Any]) -> None:
    assert run([]) == 1


def test_update_then_check_passes(env: dict[str, Any], capsys: pytest.CaptureFixture[str]) -> None:
    assert run(["--update"]) == 0
    assert json.loads(check_snapshot.VERSIONS_FILE.read_text()) == {"demo": 1}
    assert run(["demo"]) == 0
    assert "OK" in capsys.readouterr().out


def test_check_reports_drift(env: dict[str, Any], capsys: pytest.CaptureFixture[str]) -> None:
    run(["--update"])
    env["current"] = [
        *replace(BASE, fn("is_attester", [ADDR], "u32"))[1:],
        fn("new_fn", []),
    ]
    assert run([]) == 1
    out = capsys.readouterr().out
    assert "- removed function_v0: is_attester" in out
    assert "+ added   function_v0: new_fn" in out
    assert "breaking change: bump INTERFACE_VERSION" in out


def test_check_reports_changed_entry(
    env: dict[str, Any], capsys: pytest.CaptureFixture[str]
) -> None:
    run(["--update"])
    env["current"] = replace(BASE, fn("pause", [ADDR]))
    assert run([]) == 1
    assert "~ changed function_v0: pause" in capsys.readouterr().out


def test_check_fails_when_version_not_recorded(
    env: dict[str, Any], capsys: pytest.CaptureFixture[str]
) -> None:
    run(["--update"])
    set_version(env, 2)
    assert run([]) == 1
    assert "INTERFACE_VERSION is 2 but the snapshot records 1" in capsys.readouterr().out


def test_update_refuses_breaking_change_without_bump(
    env: dict[str, Any], capsys: pytest.CaptureFixture[str]
) -> None:
    run(["--update"])
    env["current"] = BASE[1:]
    assert run(["--update"]) == 1
    assert "refusing to update" in capsys.readouterr().out


def test_update_accepts_breaking_change_with_bump(env: dict[str, Any]) -> None:
    run(["--update"])
    env["current"] = BASE[1:]
    set_version(env, 2)
    assert run(["--update"]) == 0
    assert run([]) == 0


def test_update_accepts_additive_change_without_bump(env: dict[str, Any]) -> None:
    run(["--update"])
    env["current"] = [*BASE, fn("new_fn", [])]
    assert run(["--update"]) == 0


def test_unknown_contract(env: dict[str, Any]) -> None:
    with pytest.raises(SystemExit, match="unknown contract"):
        check_snapshot.main(["nope"])


def test_missing_interface_version(env: dict[str, Any]) -> None:
    (env["crate"] / "src" / "lib.rs").write_text("")
    with pytest.raises(ValueError, match="INTERFACE_VERSION"):
        check_snapshot.main(["--update"])


def test_repo_contracts_declare_interface_version() -> None:
    for cfg in CONTRACTS.values():
        assert check_snapshot.code_interface_version(cfg["crate_dir"]) >= 1
