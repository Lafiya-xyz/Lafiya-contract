#!/usr/bin/env python3
"""Write a deterministic synthetic event export (events.jsonl, attesters.json)
covering ledgers 1000-60000 for tests and the CI reproducibility check.
No real people or records: addresses and hashes are derived from a seed.
Ledgers are spaced 60 s apart (not 5 s) so the data spans several weeks."""

import datetime as dt
import hashlib
import json
import os
import random

HERE = os.path.dirname(os.path.abspath(__file__))
REG, ATT = "C" + "R" * 55, "C" + "A" * 55
POOL = "C" + "P" * 55
GENESIS = dt.datetime(2026, 9, 1, tzinfo=dt.timezone.utc)
REGIONS = ["lagos", "kano", "oyo", "rivers", "fct"]


def h(*parts):
    return hashlib.sha256("/".join(map(str, parts)).encode()).hexdigest()


def closed_at(ledger):
    return (GENESIS + dt.timedelta(seconds=60 * ledger)).strftime("%Y-%m-%dT%H:%M:%SZ")


def ev(ledger, contract, name, topics, data=""):
    return {"ledger": ledger, "contract": contract, "name": name, "topics": topics,
            "data": data, "tx_hash": h("tx", ledger, name, *topics), "closed_at": closed_at(ledger)}


def main():
    rnd = random.Random(20260924)
    attesters = {f"G{h('att', i)[:55].upper()}": REGIONS[i % len(REGIONS)] for i in range(12)}
    events = [ev(1000, REG, "initialized", ["GADMIN"]), ev(1001, ATT, "initialized", ["GADMIN"])]
    for i, a in enumerate(attesters):
        events.append(ev(1100 + i, REG, "attester_added", [a]))
    events.append(ev(20000, REG, "attester_suspended", [list(attesters)[3]]))
    events.append(ev(30000, REG, "attester_removed", [list(attesters)[4]]))
    events.append(ev(35000, REG, "upgraded", ["ab" * 32]))
    records = []
    ledger = 2000
    while ledger < 59000:
        a = rnd.choice(list(attesters))
        if records and rnd.random() < 0.15:
            rec = rnd.choice(records)
        else:
            rec = h("record", ledger)
            records.append(rec)
        events.append(ev(ledger, ATT, "attestation_recorded", [rec], a))
        if rnd.random() < 0.03:
            events.append(ev(ledger + 1, ATT, "attestation_revoked", [rnd.choice(records)]))
        ledger += rnd.randint(20, 90)
    pool_events = [
        ev(40000, POOL, "initialized", ["GADMIN"]),
        ev(40010, POOL, "programme_funded", ["GDONORA", "kano_mch"], "50000"),
        ev(40020, POOL, "programme_funded", ["GDONORB", "kano_mch"], "25000"),
        ev(40030, POOL, "programme_funded", ["GDONORA", "lagos_imm"], "40000"),
    ]
    for i, a in enumerate(list(attesters)[:6]):
        item = h("item", i)
        programme = "kano_mch" if i % 2 == 0 else "lagos_imm"
        pool_events.append(ev(41000 + 10 * i, POOL, "work_item_approved", [item], a))
        pool_events.append(ev(41001 + 10 * i, POOL, "work_item_programme_linked", [item, programme]))
        pool_events.append(ev(45000 + 10 * i, POOL, "payout_claimed", [item, a], "5000"))
    pool_events.append(ev(46000, POOL, "programme_refunded", ["GDONORB", "kano_mch"], "10000"))
    events.extend(pool_events)
    events.append(ev(59500, ATT, "paused", ["GADMIN"]))
    with open(os.path.join(HERE, "events.jsonl"), "w") as f:
        for e in sorted(events, key=lambda e: e["ledger"]):
            f.write(json.dumps(e, sort_keys=True) + "\n")
    with open(os.path.join(HERE, "attesters.json"), "w") as f:
        json.dump(attesters, f, indent=2, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main()
