"""Deterministic LRC-1 salt derivation (HKDF-SHA256) — Python reference.

Mirrors ``crates/lafiya-commitment/src/salt.rs`` byte for byte. The scheme is
specified in ``docs/adr/0018-deterministic-salt-derivation.md``::

    prk  = HKDF-Extract(salt=EXTRACT_SALT, ikm=master_secret)
    info = INFO_TAG || DERIVATION_V1 || u32be(len(record_id)) || record_id
           || u32be(salt_version)
    salt = HKDF-Expand(prk, info, 32)

    fingerprint = HKDF-Expand(prk, FINGERPRINT_INFO, 8)

Run the cross-language vectors with::

    python salt_test.py

Standard library only (hmac, hashlib, struct).
"""

from __future__ import annotations

import hashlib
import hmac
import struct

EXTRACT_SALT = b"lafiya:salt-derivation:v1"
INFO_TAG = b"lafiya:record-salt"
FINGERPRINT_INFO = b"lafiya:master-secret-fingerprint"
DERIVATION_V1 = 0x01

SALT_LEN = 32
FINGERPRINT_LEN = 8
MIN_MASTER_SECRET_LEN = 32
MAX_MASTER_SECRET_LEN = 64
MAX_RECORD_ID_LEN = 256


class SaltDerivationError(ValueError):
    """Inputs outside the ranges the derivation accepts."""


def hkdf_extract(salt: bytes, ikm: bytes) -> bytes:
    """RFC 5869 HKDF-Extract with HMAC-SHA256."""
    return hmac.new(salt, ikm, hashlib.sha256).digest()


def hkdf_expand(prk: bytes, info: bytes, length: int) -> bytes:
    """RFC 5869 HKDF-Expand with HMAC-SHA256."""
    if length > 255 * 32:
        raise SaltDerivationError("HKDF-SHA256 output is limited to 8160 bytes")
    okm = b""
    block = b""
    counter = 1
    while len(okm) < length:
        block = hmac.new(prk, block + info + bytes([counter]), hashlib.sha256).digest()
        okm += block
        counter += 1
    return okm[:length]


def _prk(master_secret: bytes) -> bytes:
    if not MIN_MASTER_SECRET_LEN <= len(master_secret) <= MAX_MASTER_SECRET_LEN:
        raise SaltDerivationError(
            f"master secret must be {MIN_MASTER_SECRET_LEN}..={MAX_MASTER_SECRET_LEN} bytes, "
            f"got {len(master_secret)}"
        )
    return hkdf_extract(EXTRACT_SALT, master_secret)


def salt_info(record_id: bytes, salt_version: int) -> bytes:
    """The HKDF ``info`` for one record and salt version."""
    if not 1 <= len(record_id) <= MAX_RECORD_ID_LEN:
        raise SaltDerivationError(
            f"record id must be 1..={MAX_RECORD_ID_LEN} bytes, got {len(record_id)}"
        )
    if not 1 <= salt_version <= 0xFFFFFFFF:
        raise SaltDerivationError(f"salt version must be 1..=2^32-1, got {salt_version}")
    return (
        INFO_TAG
        + bytes([DERIVATION_V1])
        + struct.pack(">I", len(record_id))
        + record_id
        + struct.pack(">I", salt_version)
    )


def derive_salt(master_secret: bytes, record_id: bytes, salt_version: int) -> bytes:
    """The 32-byte LRC-1 salt for ``record_id`` at ``salt_version``."""
    info = salt_info(record_id, salt_version)
    return hkdf_expand(_prk(master_secret), info, SALT_LEN)


def master_secret_fingerprint(master_secret: bytes) -> bytes:
    """Public 8-byte id of a master secret, for telling rotated secrets apart."""
    return hkdf_expand(_prk(master_secret), FINGERPRINT_INFO, FINGERPRINT_LEN)
