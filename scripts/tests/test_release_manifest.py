"""Tests for generate_release_manifest.py, validate_release_manifest.py and
check_manifest_compatibility.py."""

import copy
import json
import pathlib
from typing import Any

import pytest

import check_manifest_compatibility as compat
import generate_release_manifest as gen
import validate_release_manifest as validate

EXAMPLES = gen.ROOT / "docs" / "release-manifest" / "examples"
MANIFEST: dict[str, Any] = json.loads((EXAMPLES / "v0.1.0-dev.json").read_text())
REQUIREMENTS: dict[str, Any] = json.loads((EXAMPLES / "lafiya-web.requirements.json").read_text())


def write(tmp_path: pathlib.Path, name: str, data: Any) -> pathlib.Path:
    path = tmp_path / name
    path.write_text(json.dumps(data))
    return path


def run_main(monkeypatch: pytest.MonkeyPatch, main: Any, *argv: object) -> Any:
    monkeypatch.setattr("sys.argv", ["prog", *map(str, argv)])
    return main()


# --- validate ---------------------------------------------------------------


def test_example_manifest_is_valid(tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch) -> None:
    path = write(tmp_path, "m.json", MANIFEST)
    assert run_main(monkeypatch, validate.main, path) == 0


def test_schema_invalid_manifest(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    bad = copy.deepcopy(MANIFEST)
    bad["manifest_version"] = "one"
    del bad["release"]
    assert run_main(monkeypatch, validate.main, write(tmp_path, "m.json", bad)) == 1
    assert "schema error" in capsys.readouterr().err


def test_stale_binding_is_a_consistency_error() -> None:
    manifest = {
        "contracts": [{"name": "a", "wasm": {"sha256": "11"}}],
        "bindings": [
            {"contract": "a", "generated_from_wasm_sha256": "22"},
            {"contract": "b", "generated_from_wasm_sha256": "33"},
        ],
    }
    errors = validate.cross_check(manifest)
    assert len(errors) == 1 and "binding is stale" in errors[0]


def test_consistency_error_fails_main(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bad = copy.deepcopy(MANIFEST)
    bad["contracts"][0]["wasm"]["sha256"] = "a" * 64
    binding = next(b for b in bad["bindings"] if b["contract"] == bad["contracts"][0]["name"])
    binding["generated_from_wasm_sha256"] = "b" * 64
    assert run_main(monkeypatch, validate.main, write(tmp_path, "m.json", bad)) == 1


# --- compatibility ----------------------------------------------------------


@pytest.mark.parametrize(
    ("range_str", "lo", "hi"),
    [
        ("^1.2.3", (1, 2, 3), (2, 0, 0)),
        ("^0.2.3", (0, 2, 3), (0, 3, 0)),
        ("^0.0.3", (0, 0, 3), (0, 0, 4)),
        ("^0.0.0", (0, 0, 0), (0, 0, 1)),
    ],
)
def test_parse_caret_range(range_str: str, lo: tuple[int, ...], hi: tuple[int, ...]) -> None:
    assert compat.parse_caret_range(range_str) == (lo, hi)


@pytest.mark.parametrize("bad", ["1.2.3", "~1.2.3", "^1.2", ">=1.0.0"])
def test_parse_caret_range_rejects_other_syntax(bad: str) -> None:
    with pytest.raises(ValueError, match="unsupported version range"):
        compat.parse_caret_range(bad)


@pytest.mark.parametrize(
    ("version", "range_str", "ok"),
    [
        ("1.9.9", "^1.2.3", True),
        ("2.0.0", "^1.2.3", False),
        ("1.2.2", "^1.2.3", False),
        ("0.2.9", "^0.2.3", True),
        ("0.3.0", "^0.2.3", False),
        ("0.0.3", "^0.0.3", True),
        ("0.0.4", "^0.0.3", False),
        ("1.2.3-rc.1", "^1.2.3", True),
    ],
)
def test_version_in_range(version: str, range_str: str, ok: bool) -> None:
    assert compat.version_in_range(version, range_str) is ok


def test_parse_version_rejects_garbage() -> None:
    with pytest.raises(ValueError, match="unsupported version syntax"):
        compat.parse_version("latest")


def test_example_is_compatible() -> None:
    assert compat.check(MANIFEST, REQUIREMENTS) == []


def test_compatibility_matrix_violations() -> None:
    manifest = {
        "contracts": [
            {"name": "a", "storage_schema_version": 1},
            {"name": "b", "storage_schema_version": None},
            {"name": "c", "storage_schema_version": 3},
        ],
        "bindings": [{"contract": "a", "package_version": "2.0.0"}],
        "events": [
            {"contract": "a", "name": "Broken", "compatibility": "breaking"},
            {"contract": "a", "name": "Gone", "compatibility": "removed"},
            {"contract": "a", "name": "Fine", "compatibility": "compatible"},
        ],
    }
    requirements = {
        "contracts": {
            "a": {"min_storage_schema_version": 2, "binding_version_range": "^1.0.0"},
            "b": {"min_storage_schema_version": 1},
            "c": {"binding_version_range": "^1.0.0"},
            "missing": {},
        },
        "events": [
            {"contract": "a", "name": "Broken"},
            {"contract": "a", "name": "Gone"},
            {"contract": "a", "name": "Fine"},
            {"contract": "a", "name": "Absent"},
        ],
    }
    errors = compat.check(manifest, requirements)
    assert errors == [
        "a: requires storage_schema_version >= 2, manifest has 1",
        "a: requires bindings version ^1.0.0, manifest has 2.0.0",
        "b: requires storage_schema_version >= 1, manifest has None",
        "c: requires bindings, manifest has none",
        "missing: manifest has no such contract",
        "a.Broken: manifest reports a breaking change since the consumer's last checked release",
        "a.Gone: required event was removed",
        "a.Absent: required event missing from manifest",
    ]


def test_compatibility_main(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    manifest = write(tmp_path, "m.json", MANIFEST)
    ok = write(tmp_path, "ok.json", REQUIREMENTS)
    bad = write(tmp_path, "bad.json", {"repo": "x", "contracts": {"nope": {}}})
    assert run_main(monkeypatch, compat.main, manifest, ok) == 0
    assert run_main(monkeypatch, compat.main, manifest, bad) == 1
    assert "INCOMPATIBLE: x" in capsys.readouterr().err


# --- generate ---------------------------------------------------------------


def test_parse_events_splits_topics_and_data() -> None:
    events = gen.parse_events("contracts/attester-registry", "attester-registry")
    added = next(e for e in events if e["name"] == "AttesterAdded")
    assert added["topic_fields"] == ["attester"]


def test_classify_events() -> None:
    def ev(name: str, topics: list[str], data: list[str]) -> dict[str, Any]:
        return {"contract": "c", "name": name, "topic_fields": topics, "data_fields": data}

    previous = {
        ("c", "Same"): ev("Same", ["a"], ["x"]),
        ("c", "Grew"): ev("Grew", ["a"], ["x"]),
        ("c", "Topic"): ev("Topic", ["a"], []),
        ("c", "Shrunk"): ev("Shrunk", ["a"], ["x", "y"]),
    }
    events = [
        ev("Same", ["a"], ["x"]),
        ev("Grew", ["a"], ["x", "y"]),
        ev("Topic", ["b"], []),
        ev("Shrunk", ["a"], ["x"]),
        ev("New", [], []),
    ]
    got = {e["name"]: e["compatibility"] for e in gen.classify_events(events, previous)}
    assert got == {
        "Same": "compatible",
        "Grew": "compatible",
        "Topic": "breaking",
        "Shrunk": "breaking",
        "New": "new",
    }
    assert gen.classify_events([ev("X", [], [])], {})[0]["compatibility"] == "unclassified"


def test_build_events_reports_removed() -> None:
    previous = {
        "events": [
            {
                "contract": "attester-registry",
                "name": "Retired",
                "topic_fields": [],
                "data_fields": [],
            }
        ]
    }
    events = gen.build_events(previous)
    assert {"contract": "attester-registry", "name": "Retired"}.items() <= next(
        e for e in events if e["name"] == "Retired"
    ).items()
    assert next(e for e in events if e["name"] == "Retired")["compatibility"] == "removed"


def test_wasm_info_hashes_built_wasm(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(gen, "WASM_DIR", tmp_path)
    assert gen.wasm_info("missing")["sha256"] is None
    (tmp_path / "demo.wasm").write_bytes(b"abc")
    info = gen.wasm_info("demo")
    assert info["size_bytes"] == 3
    assert info["sha256"].startswith("ba7816bf")


def test_generated_manifest_is_schema_valid(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    previous = write(tmp_path, "prev.json", MANIFEST)
    out = tmp_path / "manifest.json"
    run_main(monkeypatch, gen.main, "--pretty", "--previous", previous, "-o", out)
    manifest = json.loads(out.read_text())
    assert {c["name"] for c in manifest["contracts"]} == set(gen.CONTRACTS)
    assert run_main(monkeypatch, validate.main, out) == 0


def test_generate_prints_to_stdout(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    run_main(monkeypatch, gen.main)
    assert json.loads(capsys.readouterr().out)["manifest_version"] == 1
