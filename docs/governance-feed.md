# Governance transparency feed

`dashboard/governance/feed.py` turns the public event export (same input as the
transparency dashboard) into a plain-language feed of privileged actions:
admin transfers, upgrades, pause/unpause, rate-limit and registry changes, and
attester lifecycle changes. No data beyond public chain events is used.

```bash
python3 dashboard/governance/feed.py --events events.jsonl --out site/governance \
  --explorer https://stellar.expert/explorer/public/tx [--locale fr] [--sign-key key.pem]
```

## Format (`lafiya-governance-feed/1`)

`feed.json`: `{format, locale, ledger_range, entries[]}`; each entry has `id`,
`ledger`, `closed_at`, `contract`, `event`, `summary`, `tx_hash`, `tx_url`.
`feed.atom` carries the same entries as Atom. Breaking changes bump the
trailing version. Summaries come from `dashboard/governance/messages.json`
(English fallback for untranslated keys).

## Verification

1. Re-export the events for `ledger_range` and run the script with `--verify`
   (`--out` pointing at the published files). It regenerates the feed from the
   chain events and fails on any difference, including `digest.json`.
2. Check the signature (Ed25519):
   `openssl pkeyutl -verify -rawin -pubin -inkey pub.pem -in digest.json -sigfile digest.sig`
   `digest.json` holds the sha256 of `feed.json`.
