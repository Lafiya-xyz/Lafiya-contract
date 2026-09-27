"""Lafiya Record Commitment v1 (LRC-1) — Python reference implementation.

This mirrors ``crates/lafiya-commitment/src/lib.rs`` field for field, byte for
byte. See the crate README and ``docs/adr/0008-record-commitment-canonicalization.md``
for the specification this implements.

Run the test suite with::

    python lrc1_test.py

No third-party dependencies; standard library only (hashlib, struct, json, pathlib).
Python 3.8+ is required.

Encoding summary
----------------
``commitment = SHA-256(DOMAIN_TAG || VERSION_V1 || canonical_payload)``

Each field encodes as a tag byte followed by its value:

=======  =========  ============================================================
Tag      Meaning    Encoding
=======  =========  ============================================================
0x00     Absent     tag only
0x01     Null       tag only
0x10     Text       u32 big-endian byte length + UTF-8 bytes (caller must NFC)
0x11     Int64      8-byte big-endian two's-complement signed integer
0x12     Bool       one byte, 0x00 (False) or 0x01 (True)
0x13     Bytes      u32 big-endian byte length + raw bytes
=======  =========  ============================================================
"""

from __future__ import annotations

import hashlib
import struct
from dataclasses import dataclass
from typing import Union

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

DOMAIN_TAG: bytes = b"lafiya:record-commitment"
"""Domain-separation tag mixed into every LRC-1 commitment."""

VERSION_V1: int = 0x01
"""Canonicalization scheme version byte for LRC-1."""

VERSION_LEGACY_UNVERSIONED: int = 0x00
"""Reserved version byte for pre-LRC-1 commitments; never produced by this implementation."""

_TAG_ABSENT: int = 0x00
_TAG_NULL: int = 0x01
_TAG_TEXT: int = 0x10
_TAG_INT64: int = 0x11
_TAG_BOOL: int = 0x12
_TAG_BYTES: int = 0x13

# ---------------------------------------------------------------------------
# Field value types
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Absent:
    """The field is not present in the source record."""


@dataclass(frozen=True)
class Null:
    """The field is present with an explicit null value."""


@dataclass(frozen=True)
class Text:
    """A UTF-8 string. Callers MUST supply Unicode Normalization Form C (NFC)."""

    value: str


@dataclass(frozen=True)
class Int64:
    """A signed 64-bit integer. Stored as a string to avoid silent precision loss
    on platforms where Python ints happen to be narrower (though CPython always
    uses arbitrary precision). The constructor validates the range.
    """

    value: int

    def __post_init__(self) -> None:
        lo = -(2**63)
        hi = (2**63) - 1
        if not (lo <= self.value <= hi):
            raise ValueError(
                f"Int64 value {self.value!r} out of range [{lo}, {hi}]"
            )


@dataclass(frozen=True)
class Bool:
    """A boolean flag."""

    value: bool


@dataclass(frozen=True)
class Bytes:
    """Raw bytes, e.g. a pre-hashed or salted opaque reference."""

    value: bytes


FieldValue = Union[Absent, Null, Text, Int64, Bool, Bytes]
"""A single record field in its fixed schema position."""

# ---------------------------------------------------------------------------
# Encoding
# ---------------------------------------------------------------------------


def _encode_field(field: FieldValue) -> bytes:
    if isinstance(field, Absent):
        return bytes([_TAG_ABSENT])
    elif isinstance(field, Null):
        return bytes([_TAG_NULL])
    elif isinstance(field, Text):
        utf8 = field.value.encode("utf-8")
        return bytes([_TAG_TEXT]) + struct.pack(">I", len(utf8)) + utf8
    elif isinstance(field, Int64):
        return bytes([_TAG_INT64]) + struct.pack(">q", field.value)
    elif isinstance(field, Bool):
        return bytes([_TAG_BOOL]) + bytes([1 if field.value else 0])
    elif isinstance(field, Bytes):
        return bytes([_TAG_BYTES]) + struct.pack(">I", len(field.value)) + field.value
    else:
        raise TypeError(f"Unknown FieldValue type: {type(field)!r}")


def encode_payload(fields: list[FieldValue]) -> bytes:
    """Encode an ordered list of record fields into the LRC-1 canonical payload.

    Field order is fixed by the schema, not sorted — callers must always pass
    fields in the same schema-defined order.
    """
    out = bytearray()
    for field in fields:
        out += _encode_field(field)
    return bytes(out)


def commit_v1(fields: list[FieldValue]) -> bytes:
    """Compute the LRC-1 record commitment:
    ``SHA-256(DOMAIN_TAG || VERSION_V1 || canonical_payload)``.

    Returns 32 bytes.
    """
    payload = encode_payload(fields)
    h = hashlib.sha256()
    h.update(DOMAIN_TAG)
    h.update(bytes([VERSION_V1]))
    h.update(payload)
    return h.digest()
