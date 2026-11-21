#!/usr/bin/env python3
"""Generate docs/specs/vectors/dispute-v2-vectors.json (docs/specs/dispute-signature-v2.md).

Synthetic inputs only. Stdlib only. Run: python3 scripts/gen_dispute_vectors.py
"""
import base64
import hashlib
import json
import struct
from pathlib import Path

DOMAIN = b"lafiya.attestation.dispute.v2"
OUT = Path(__file__).resolve().parent.parent / "docs/specs/vectors/dispute-v2-vectors.json"


def crc16_xmodem(data: bytes) -> int:
    crc = 0
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) if crc & 0x8000 else crc << 1
            crc &= 0xFFFF
    return crc


def strkey(version: int, payload: bytes) -> str:
    body = bytes([version]) + payload
    return base64.b32encode(body + struct.pack("<H", crc16_xmodem(body))).decode()


def account(key: bytes) -> tuple[str, bytes]:
    # ScVal::Address(ScAddress::Account(PublicKey::Ed25519(key)))
    return strkey(6 << 3, key), struct.pack(">III", 18, 0, 0) + key


def contract(cid: bytes) -> tuple[str, bytes]:
    # ScVal::Address(ScAddress::Contract(cid))
    return strkey(2 << 3, cid), struct.pack(">II", 18, 1) + cid


def vector(name, network, cid, record_hash, key, nonce, expires_at):
    network_id = hashlib.sha256(network.encode()).digest()
    c_str, c_xdr = contract(cid)
    s_str, s_xdr = account(key)
    preimage = (
        DOMAIN + network_id + c_xdr + record_hash + s_xdr + key
        + struct.pack(">QQ", nonce, expires_at)
    )
    return {
        "name": name,
        "network_passphrase": network,
        "network_id_hex": network_id.hex(),
        "contract": c_str,
        "record_hash_hex": record_hash.hex(),
        "signer": s_str,
        "signer_key_hex": key.hex(),
        "nonce": str(nonce),
        "expires_at": str(expires_at),
        "preimage_hex": preimage.hex(),
        "message_hex": hashlib.sha256(preimage).hexdigest(),
    }


TESTNET = "Test SDF Network ; September 2015"
PUBNET = "Public Global Stellar Network ; September 2015"
base = dict(cid=bytes(range(32)), record_hash=bytes([0xAB] * 32), key=bytes([0x11] * 32))
vectors = [
    vector("testnet-basic", TESTNET, nonce=1, expires_at=1_700_000_000, **base),
    vector("pubnet-same-inputs", PUBNET, nonce=1, expires_at=1_700_000_000, **base),
    vector("other-contract", TESTNET, nonce=1, expires_at=1_700_000_000,
           **{**base, "cid": bytes([0xFF] * 32)}),
    vector("max-nonce", TESTNET, nonce=2**64 - 1, expires_at=2**64 - 1, **base),
]
OUT.write_text(json.dumps({"domain": DOMAIN.decode(), "vectors": vectors}, indent=2) + "\n")
print(f"wrote {OUT}")
