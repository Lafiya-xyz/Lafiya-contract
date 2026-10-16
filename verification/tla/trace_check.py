#!/usr/bin/env python3
"""Check an incentive-pool execution trace against IncentivePool.tla.

Each trace line is the JSON state after one contract call (see
`emit_pool_trace` in contracts/incentive-pool/src/test.rs). The checker
replays the spec's transition relation (Fund/Withdraw/Approve/Claim) and
its invariants (Conservation, NoOverdraw, NoDoublePay, Caps).

usage: trace_check.py TRACE.jsonl [--max-per-claim N] [--max-per-attester N]
       trace_check.py --self-test TRACE.jsonl   # mutation check
Exit 0 = conforms, 1 = violation.
"""
import json
import sys

ONE = 1_000_000


def check(lines, max_claim=50 * ONE, max_att=100_000 * ONE):
    dep = paid = 0
    approved, claims, att_of, amt = set(), {}, {}, {}
    per = [0, 0]
    for n, raw in enumerate(lines, 1):
        s = json.loads(raw)
        fail = lambda m: (n, m)
        if s["ok"]:
            op = s["op"]
            if op == "fund":
                dep += s["x"]
            elif op == "withdraw":
                if s["x"] > dep - paid:
                    return fail("Withdraw exceeds available")
                paid += s["x"]
            elif op == "approve":
                if s["item"] in approved:
                    return fail("Approve of already-approved item")
                if s["x"] > max_claim:
                    return fail("Approve above MaxPerClaim")
                approved.add(s["item"])
                att_of[s["item"]], amt[s["item"]] = s["attester"], s["x"]
            elif op == "claim":
                i = s["item"]
                if i not in approved:
                    return fail("Claim of unapproved item")
                claims[i] = claims.get(i, 0) + 1
                if claims[i] > 1:
                    return fail("NoDoublePay violated: item claimed twice")
                paid += amt[i]
                per[att_of[i]] += amt[i]
                if per[att_of[i]] > max_att:
                    return fail("MaxPerAttester exceeded")
        if (s["deposited"], s["paid"]) != (dep, paid):
            return fail(f"state diverges from spec: got dep={s['deposited']} paid={s['paid']}, spec dep={dep} paid={paid}")
        if s["balance"] != dep - paid or paid > dep:
            return fail("Conservation violated")
        if s["per_attester"] != per:
            return fail("AttesterAccounting violated")
    return None


def main(argv):
    mutate = "--self-test" in argv
    args = [a for a in argv[1:] if not a.startswith("--")]
    lines = [l for l in open(args[0]).read().splitlines() if l.strip()]
    err = check(lines)
    if err:
        print(f"FAIL line {err[0]}: {err[1]}")
        return 1
    print(f"trace conforms ({len(lines)} steps)")
    if mutate:
        # Mutation: replay the first successful claim a second time (double pay).
        for i, l in enumerate(lines):
            s = json.loads(l)
            if s["op"] == "claim" and s["ok"]:
                dup = dict(s, paid=s["paid"] + (s["paid"] - json.loads(lines[i - 1])["paid"]))
                dup["balance"] -= dup["paid"] - s["paid"]
                if check(lines[: i + 1] + [json.dumps(dup)]) is None:
                    print("MUTATION NOT DETECTED")
                    return 1
                print("injected double-pay mutation detected")
                return 0
        print("no successful claim in trace; cannot mutate")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
