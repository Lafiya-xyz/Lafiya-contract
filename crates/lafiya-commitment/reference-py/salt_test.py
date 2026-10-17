"""Checks the Python salt derivation against ``vectors/salt-derivation-vectors.json``.

The same fixture is checked by ``src/salt.rs`` (cargo test) and
``reference-ts/salt.test.ts``, so all three implementations agree byte for
byte. Run with::

    python salt_test.py
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

_THIS_DIR = Path(__file__).parent
sys.path.insert(0, str(_THIS_DIR))

from lrc1 import DOMAIN_TAG, VERSION_V1, Bytes, FieldValue, Int64, Text, encode_payload  # noqa: E402
from salt import (  # noqa: E402
    SaltDerivationError,
    derive_salt,
    hkdf_expand,
    hkdf_extract,
    master_secret_fingerprint,
    salt_info,
)

VECTORS = _THIS_DIR.parent / "vectors" / "salt-derivation-vectors.json"


def _field(f: dict) -> FieldValue:
    if f["kind"] == "text":
        return Text(f["value"])
    if f["kind"] == "int64":
        return Int64(int(f["value"]))
    raise ValueError(f"unsupported field kind in salt vectors: {f['kind']}")


def main() -> int:
    doc = json.loads(VECTORS.read_text(encoding="utf-8"))

    for tc in doc["hkdf_rfc5869"]:
        prk = hkdf_extract(bytes.fromhex(tc["salt_hex"]), bytes.fromhex(tc["ikm_hex"]))
        okm = hkdf_expand(prk, bytes.fromhex(tc["info_hex"]), tc["length"])
        assert okm.hex() == tc["okm_hex"], f"HKDF mismatch for {tc['name']}"

    for v in doc["vectors"]:
        secret = bytes.fromhex(v["master_secret_hex"])
        record_id = v["record_id"].encode("utf-8")
        name = v["name"]
        assert salt_info(record_id, v["salt_version"]).hex() == v["info_hex"], f"info: {name}"
        salt = derive_salt(secret, record_id, v["salt_version"])
        assert salt.hex() == v["salt_hex"], f"salt: {name}"
        assert master_secret_fingerprint(secret).hex() == v["fingerprint_hex"], f"fingerprint: {name}"
        if "commitment_hex" in v:
            fields = [Bytes(salt)] + [_field(f) for f in v["fields"]]
            payload = encode_payload(fields)
            commitment = hashlib.sha256(DOMAIN_TAG + bytes([VERSION_V1]) + payload).hexdigest()
            assert commitment == v["commitment_hex"], f"commitment: {name}"

    for v in doc["invalid"]:
        try:
            derive_salt(bytes.fromhex(v["master_secret_hex"]), v["record_id"].encode(), v["salt_version"])
        except SaltDerivationError:
            continue
        raise AssertionError(f"invalid vector {v['name']} was accepted")

    salts = {v["salt_hex"] for v in doc["vectors"]}
    assert len(salts) == len(doc["vectors"]), "every vector must derive a distinct salt"

    print(
        f"ok: {len(doc['hkdf_rfc5869'])} HKDF, {len(doc['vectors'])} derivation and "
        f"{len(doc['invalid'])} invalid vectors matched"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
