"""Shared fixtures for the Python tooling tests."""

import json
import pathlib
import subprocess
from typing import Any

import pytest

FIXTURES = pathlib.Path(__file__).resolve().parent / "fixtures"


def load_fixture(name: str) -> Any:
    return json.loads((FIXTURES / name).read_text())


def completed(stdout: str = "", returncode: int = 0, stderr: str = "") -> Any:
    return subprocess.CompletedProcess([], returncode, stdout=stdout, stderr=stderr)


@pytest.fixture
def wasm(tmp_path: pathlib.Path) -> pathlib.Path:
    path = tmp_path / "contract.wasm"
    path.write_bytes(b"\0asm")
    return path
