#!/usr/bin/env python3
"""Proof of concept for docs/security/linkability.md.

    python3 scripts/poc/linkability_poc.py [--seed N]

Builds a local sandbox event log (the shape of `attestation-registry` attest
events: record hash, attester, ledger timestamp) for a population of patients
visiting attesters in different regions, then plays an observer who scanned
one patient's card once.

1. Baseline: the attested hash repeats on every re-attestation, so the
   observer recovers the patient's full visit history (when, which attester,
   which region) by filtering events on that hash.
2. Mitigation (ADR-0012, per-attestation blinded commitments): each
   attestation commits to H(domain || record || nonce) with a fresh nonce
   carried on the card, so no two events share a hash and the scanned hash
   matches only the single attestation printed on that card.

Exits non-zero if the attack does not work on the baseline or still works
under the mitigation.
"""
import argparse
import hashlib
import os
import random

DOMAIN = b"lafiya:attest:v1"
REGIONS = {"GA": "Gombe", "GB": "Bauchi", "GC": "Kano"}


def h(*parts):
    return hashlib.sha256(b"|".join(parts)).hexdigest()


def simulate(rng, blinded, patients=200, visits=6):
    attesters = [f"{code}-chw{i}" for code in REGIONS for i in range(3)]
    events, cards = [], {}
    for p in range(patients):
        record = f"patient-{p}".encode()
        home = rng.choice(list(REGIONS))
        t = rng.randrange(0, 30) * 86_400
        for _ in range(visits):
            region = home if rng.random() < 0.7 else rng.choice(list(REGIONS))
            attester = rng.choice([a for a in attesters if a.startswith(region)])
            nonce = os.urandom(16) if blinded else b""
            record_hash = h(DOMAIN, record, nonce) if blinded else h(DOMAIN, record)
            events.append({"record_hash": record_hash, "attester": attester, "ts": t})
            cards[p] = record_hash  # the card shows the latest attestation
            t += rng.randrange(7, 60) * 86_400
    return events, cards


def watch_hash(events, scanned_hash):
    """The attack: link every on-chain event carrying the scanned hash."""
    return [e for e in events if e["record_hash"] == scanned_hash]


def run(blinded, seed):
    rng = random.Random(seed)
    events, cards = simulate(rng, blinded)
    victim = rng.choice(sorted(cards))
    linked = watch_hash(events, cards[victim])
    regions = sorted({REGIONS[e["attester"][:2]] for e in linked})
    label = "blinded commitments" if blinded else "baseline (repeating hash)"
    print(f"[{label}] {len(events)} events; observer linked {len(linked)} event(s) "
          f"to patient-{victim}; regions revealed: {', '.join(regions)}")
    return len(linked)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seed", type=int, default=7)
    seed = parser.parse_args().seed
    baseline = run(blinded=False, seed=seed)
    mitigated = run(blinded=True, seed=seed)
    ok = baseline > 1 and mitigated == 1
    print("RESULT:", "attack works on baseline and is defeated by the mitigation" if ok else "UNEXPECTED")
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
