#!/usr/bin/env python3
"""Capacity and cost forecast for a national-scale Lafiya rollout.

Pure function of budgets/capacity-model.json (inputs + assumptions) and the
per-operation fees in budgets/baseline.json, so a run is reproducible.

  python3 scripts/capacity_model.py              # forecast + sensitivity
  python3 scripts/capacity_model.py --json       # machine-readable
  python3 scripts/capacity_model.py --check      # validate vs testnet_observations
"""
import argparse
import copy
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODEL = ROOT / "budgets" / "capacity-model.json"
BASELINE = ROOT / "budgets" / "baseline.json"
STROOPS_PER_XLM = 10_000_000
SECONDS_PER_DAY = 86_400


def forecast(scn, asm, fees):
    """Return outputs for one scenario (all money in stroops)."""
    ledgers_per_day = SECONDS_PER_DAY / asm["ledger_seconds"]
    ttl_ledgers = scn["ttl_days"] * ledgers_per_day
    overhead = asm["entry_overhead_bytes"]

    # Live attestation entries: one per card, history kept per card up to the window.
    entries = scn["cards"] * scn["history_window"]
    att_bytes = entries * (asm["attestation_entry_bytes"] + overhead)
    atr_bytes = scn["attesters"] * (asm["attester_entry_bytes"] + overhead)
    storage_bytes = att_bytes + atr_bytes

    # Rent: every live entry is extended once per TTL period -> yearly cost.
    extensions_per_year = 365 / scn["ttl_days"]
    rent_per_period = storage_bytes * ttl_ledgers * asm["rent_stroops_per_byte_per_ledger"]
    rent_year = rent_per_period * extensions_per_year

    # Fees: write ops (attest) and read ops (verification) per year.
    attest_fee = fees["attest"]
    verify_fee = fees["get_attestation"]
    fee_year = 365 * (
        scn["registrations_per_day"] * attest_fee
        + scn["verifications_per_day"] * verify_fee
    )

    # Indexer: every attestation event retained.
    indexer_bytes = entries * asm["indexer_bytes_per_attestation"] * asm["indexer_overhead_factor"]

    avg_qps = scn["verifications_per_day"] * scn["rpc_calls_per_verification"] / SECONDS_PER_DAY
    reg_qps = scn["registrations_per_day"] * 2 / SECONDS_PER_DAY  # simulate + send
    rpc_qps = (avg_qps + reg_qps) * scn["peak_to_average"]

    return {
        "storage_bytes": storage_bytes,
        "rent_stroops_per_year": rent_year,
        "fee_stroops_per_year": fee_year,
        "total_stroops_per_year": rent_year + fee_year,
        "indexer_bytes": indexer_bytes,
        "rpc_qps_peak": rpc_qps,
    }


def sensitivity(scn, asm, fees, delta=0.10):
    """Effect on total yearly cost of +10% on each numeric driver."""
    base = forecast(scn, asm, fees)["total_stroops_per_year"]
    rows = []
    for group, params in (("scenario", scn), ("assumptions", asm)):
        for key, val in params.items():
            if not isinstance(val, (int, float)) or isinstance(val, bool):
                continue
            s, a = copy.deepcopy(scn), copy.deepcopy(asm)
            (s if group == "scenario" else a)[key] = val * (1 + delta)
            total = forecast(s, a, fees)["total_stroops_per_year"]
            rows.append((key, (total - base) / base * 100 if base else 0.0))
    rows.sort(key=lambda r: -abs(r[1]))
    return rows


def load():
    model = json.loads(MODEL.read_text())
    baseline = json.loads(BASELINE.read_text())
    fees = {
        op: v["estimated_fee_stroops"]
        for ops in baseline.values()
        for op, v in ops.items()
        if "estimated_fee_stroops" in v
    }
    return model, fees


def check(model, fees):
    """Compare model fee predictions with recorded testnet observations."""
    obs = model["testnet_observations"]
    if not obs:
        print("no testnet_observations recorded; nothing to validate")
        return 0
    worst = 0.0
    for o in obs:
        predicted = fees[o["operation"]]
        err = abs(predicted - o["observed_fee_stroops"]) / o["observed_fee_stroops"]
        worst = max(worst, err)
        print(f"{o['operation']}: predicted={predicted} observed={o['observed_fee_stroops']} error={err:.1%}")
    return 1 if worst > model.get("tolerance", 0.25) else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    model, fees = load()
    if args.check:
        return check(model, fees)
    scn, asm = model["scenario"], model["assumptions"]
    out = forecast(scn, asm, fees)
    sens = sensitivity(scn, asm, fees)
    if args.json:
        json.dump({"model_version": model["model_version"], "outputs": out,
                   "sensitivity_pct_per_10pct": dict(sens)}, sys.stdout, indent=2)
        print()
        return 0
    print(f"model {model['model_version']}  cards={scn['cards']:,}")
    print(f"storage        {out['storage_bytes'] / 1e9:,.2f} GB")
    print(f"rent / year    {out['rent_stroops_per_year'] / STROOPS_PER_XLM:,.0f} XLM")
    print(f"fees / year    {out['fee_stroops_per_year'] / STROOPS_PER_XLM:,.0f} XLM")
    print(f"indexer size   {out['indexer_bytes'] / 1e9:,.2f} GB")
    print(f"RPC peak QPS   {out['rpc_qps_peak']:,.1f}")
    print("top cost drivers (% change in yearly cost per +10% input):")
    for key, pct in sens[:3]:
        print(f"  {key:36s}{pct:+.2f}%")
    return 0


if __name__ == "__main__":
    sys.exit(main())
