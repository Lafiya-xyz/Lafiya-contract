"""Cross-language test suite for the LRC-1 Python reference implementation.

Verifies the Python implementation against the shared fixture in
``vectors/lrc1-test-vectors.json``, then runs a randomized cross-checker that
generates N records, hashes them with both the Python and the inline reference
logic, and compares the results.

Run with::

    python lrc1_test.py [--vectors-path PATH] [--random-count N]

Default random-count is 10000.  No third-party dependencies.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import struct
import sys
import unicodedata
from pathlib import Path

# ---------------------------------------------------------------------------
# Import the reference implementation. Support running from any working dir.
# ---------------------------------------------------------------------------
_THIS_DIR = Path(__file__).parent
sys.path.insert(0, str(_THIS_DIR))

from lrc1 import (  # noqa: E402
    Absent,
    Bool,
    Bytes,
    FieldValue,
    Int64,
    Null,
    Text,
    commit_v1,
    encode_payload,
)

# ---------------------------------------------------------------------------
# Default path to vectors file
# ---------------------------------------------------------------------------
_DEFAULT_VECTORS_PATH = _THIS_DIR.parent / "vectors" / "lrc1-test-vectors.json"

# ---------------------------------------------------------------------------
# JSON deserialization helpers
# ---------------------------------------------------------------------------


def _json_field_to_field_value(f: dict) -> FieldValue:
    kind = f["kind"]
    if kind == "absent":
        return Absent()
    elif kind == "null":
        return Null()
    elif kind == "text":
        return Text(f["value"])
    elif kind == "int64":
        # JSON represents i64 as a decimal string to survive JS 53-bit limits.
        return Int64(int(f["value"]))
    elif kind == "bool":
        return Bool(f["value"])
    elif kind == "bytes":
        return Bytes(bytes.fromhex(f["value"]))
    else:
        raise ValueError(f"Unknown field kind: {kind!r}")


# ---------------------------------------------------------------------------
# Test: shared fixture vectors
# ---------------------------------------------------------------------------


def run_vector_tests(vectors_path: Path) -> int:
    """Load the shared fixture and assert Python output matches.

    Returns the number of vectors checked.
    """
    with vectors_path.open(encoding="utf-8") as fh:
        vectors = json.load(fh)

    assert len(vectors) > 0, "fixture must not be empty"

    for v in vectors:
        name = v["name"]
        fields = [_json_field_to_field_value(f) for f in v["fields"]]

        payload = encode_payload(fields)
        assert payload.hex() == v["payload_hex"], (
            f"payload mismatch for vector '{name}':\n"
            f"  got:      {payload.hex()}\n"
            f"  expected: {v['payload_hex']}"
        )

        commitment = commit_v1(fields)
        assert commitment.hex() == v["commitment_hex"], (
            f"commitment mismatch for vector '{name}':\n"
            f"  got:      {commitment.hex()}\n"
            f"  expected: {v['commitment_hex']}"
        )

    return len(vectors)


# ---------------------------------------------------------------------------
# Randomized cross-checker
# ---------------------------------------------------------------------------
#
# Generates N random records, encodes them twice (once through the public API,
# once through an inline reference that re-implements the same byte layout) and
# asserts the outputs match.  This exercises every code path without relying on
# pre-computed golden values.


_SAMPLE_STRINGS = [
    "",
    "a",
    "hello",
    "lafiya",
    "José",
    "allergy",
    "penicillin",
    "\u0000",
    "\uFEFFtest",
    "\U0001F3E5",
    "e\u0301",
    "\u00E9",
    "\u200Frtl",
    "a" * 255,
    "a" * 256,
]

_SAMPLE_BYTES_VALS = [
    b"",
    b"\xff",
    b"\x00" * 16,
    bytes(range(256)),
    b"\xaa" * 255,
    b"\xbb" * 256,
]


def _random_field(rng: random.Random) -> FieldValue:
    tag = rng.randint(0, 5)
    if tag == 0:
        return Absent()
    elif tag == 1:
        return Null()
    elif tag == 2:
        return Text(rng.choice(_SAMPLE_STRINGS))
    elif tag == 3:
        # Mix safe and unsafe i64 boundaries
        candidates = [
            -(2**63),
            -1,
            0,
            1,
            9007199254740992,    # 2^53
            9007199254740993,    # 2^53 + 1
            -9007199254740993,
            (2**63) - 1,
            rng.randint(-(2**62), (2**62)),
        ]
        return Int64(rng.choice(candidates))
    elif tag == 4:
        return Bool(rng.choice([True, False]))
    else:
        return Bytes(rng.choice(_SAMPLE_BYTES_VALS))


def _reference_encode_payload(fields: list[FieldValue]) -> bytes:
    """Inline re-implementation of encode_payload, intentionally written
    separately to catch copy-paste errors in the primary implementation."""
    TAG_ABSENT = 0x00
    TAG_NULL   = 0x01
    TAG_TEXT   = 0x10
    TAG_INT64  = 0x11
    TAG_BOOL   = 0x12
    TAG_BYTES  = 0x13

    out = bytearray()
    for f in fields:
        if isinstance(f, Absent):
            out += bytes([TAG_ABSENT])
        elif isinstance(f, Null):
            out += bytes([TAG_NULL])
        elif isinstance(f, Text):
            utf8 = f.value.encode("utf-8")
            out += bytes([TAG_TEXT])
            out += struct.pack(">I", len(utf8))
            out += utf8
        elif isinstance(f, Int64):
            out += bytes([TAG_INT64])
            out += struct.pack(">q", f.value)
        elif isinstance(f, Bool):
            out += bytes([TAG_BOOL])
            out += bytes([1 if f.value else 0])
        elif isinstance(f, Bytes):
            out += bytes([TAG_BYTES])
            out += struct.pack(">I", len(f.value))
            out += f.value
    return bytes(out)


def _reference_commit_v1(fields: list[FieldValue]) -> bytes:
    payload = _reference_encode_payload(fields)
    h = hashlib.sha256()
    h.update(b"lafiya:record-commitment")
    h.update(bytes([0x01]))
    h.update(payload)
    return h.digest()


def run_random_cross_check(n: int, seed: int = 42) -> int:
    """Generate *n* random records and assert the public API matches the
    inline reference.  Returns *n* (the number of records checked).
    """
    rng = random.Random(seed)

    for i in range(n):
        num_fields = rng.randint(0, 8)
        fields = [_random_field(rng) for _ in range(num_fields)]

        payload_api = encode_payload(fields)
        payload_ref = _reference_encode_payload(fields)
        assert payload_api == payload_ref, (
            f"random record {i}: payload mismatch\n"
            f"  fields: {fields}\n"
            f"  api: {payload_api.hex()}\n"
            f"  ref: {payload_ref.hex()}"
        )

        commitment_api = commit_v1(fields)
        commitment_ref = _reference_commit_v1(fields)
        assert commitment_api == commitment_ref, (
            f"random record {i}: commitment mismatch\n"
            f"  fields: {fields}\n"
            f"  api: {commitment_api.hex()}\n"
            f"  ref: {commitment_ref.hex()}"
        )

    return n


# ---------------------------------------------------------------------------
# Structural property assertions
# ---------------------------------------------------------------------------


def run_property_tests() -> None:
    """Assert structural properties that every correct implementation must satisfy."""

    # Absent / Null / Text("") / Bytes([]) must all produce different commitments.
    c_absent    = commit_v1([Absent()])
    c_null      = commit_v1([Null()])
    c_empty_txt = commit_v1([Text("")])
    c_empty_byt = commit_v1([Bytes(b"")])
    assert len({c_absent, c_null, c_empty_txt, c_empty_byt}) == 4, (
        "Absent / Null / Text('') / Bytes([]) must all differ"
    )

    # NFC and NFD forms of the same grapheme must differ (caller must normalize).
    nfc = "\u00E9"            # é precomposed
    nfd = "e\u0301"           # e + combining acute
    assert unicodedata.normalize("NFC", nfc) != unicodedata.normalize("NFC", nfd) or \
           nfc.encode("utf-8") != nfd.encode("utf-8"), "test setup: NFC/NFD must differ in bytes"
    assert commit_v1([Text(nfc)]) != commit_v1([Text(nfd)]), (
        "NFC and NFD must produce different commitments"
    )

    # Naive-concatenation collision: ["ab","c"] vs ["a","bc"]
    left  = commit_v1([Text("ab"), Text("c")])
    right = commit_v1([Text("a"),  Text("bc")])
    assert left != right, "length-prefix must prevent concatenation collision"

    # Field order is significant.
    fwd = commit_v1([Text("first"), Text("second")])
    rev = commit_v1([Text("second"), Text("first")])
    assert fwd != rev, "field order must be significant"

    # i64 boundary values must all produce distinct commitments.
    boundaries = [
        Int64(-(2**63)),
        Int64(-1),
        Int64(0),
        Int64(1),
        Int64(9007199254740992),
        Int64(9007199254740993),
        Int64((2**63) - 1),
    ]
    hashes = {commit_v1([b]) for b in boundaries}
    assert len(hashes) == len(boundaries), "i64 boundary values must all differ"

    # Commitment is deterministic.
    fields = [Bool(True), Int64(42)]
    assert commit_v1(fields) == commit_v1(fields), "commitment must be deterministic"


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(description="LRC-1 Python reference test suite")
    parser.add_argument(
        "--vectors-path",
        type=Path,
        default=_DEFAULT_VECTORS_PATH,
        help="Path to lrc1-test-vectors.json (default: relative to this file)",
    )
    parser.add_argument(
        "--random-count",
        type=int,
        default=10_000,
        help="Number of random records for the cross-checker (default: 10000)",
    )
    args = parser.parse_args()

    vectors_path: Path = args.vectors_path
    random_count: int = args.random_count

    print(f"Loading vectors from: {vectors_path}")
    n_vectors = run_vector_tests(vectors_path)
    print(f"ok: {n_vectors} fixture vectors matched")

    print("Running property tests...")
    run_property_tests()
    print("ok: property tests passed")

    print(f"Running {random_count:,} random cross-checks...")
    run_random_cross_check(random_count)
    print(f"ok: {random_count:,} random records matched")


if __name__ == "__main__":
    main()
