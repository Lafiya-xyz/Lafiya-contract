# Verifying a published dashboard snapshot

Every build writes `data/snapshot.json`:

| Field | Meaning |
| --- | --- |
| `ledger_height` | Exclusive upper ledger of the data (`--ledger-to`) |
| `index_version` | SHA-256 of the canonical JSON of the source events used |
| `content_hash` | SHA-256 over `sha256  filename` lines of every other file in `data/` |

`data/snapshot.sig` is an Ed25519 signature over `snapshot.json`, made by the
project key in CI (secret `LAFIYA_DASHBOARD_SIGNING_KEY`, generated with
`openssl genpkey -algorithm ed25519`). The maintainers commit the public key as
`dashboard/signing-key.pub` when the key is first generated; compare it with the copy in the release notes.

## Verify

1. Download `data/` from the site and export the same events:
   `lafiya-watchdog --config watchdog.toml export --from <ledger_from> --to <ledger_to> > events.jsonl`
   (the command is in `data/provenance.json`).
2. Check the signature, the content hash, and recompute everything from public data:

```bash
python3 dashboard/snapshot.py verify --data data --pubkey dashboard/signing-key.pub \
  --regenerate -- --events events.jsonl --attesters attesters.json --network <net> \
  --ledger-from <L1> --ledger-to <L2> --contracts <C1,C2>
```

Exit code 0 prints `snapshot OK`. Needs Python 3.8+ and OpenSSL 3.x only.

## Key rotation

1. Generate a new key; store the private PEM in `LAFIYA_DASHBOARD_SIGNING_KEY`.
2. Replace `dashboard/signing-key.pub` in a reviewed PR and announce the new
   fingerprint (`openssl pkey -pubin -in signing-key.pub -outform der | sha256sum`).
3. Keep old public keys in `dashboard/signing-key.<year>.pub`; snapshots signed
   before rotation verify against the key that was current at their ledger height.
4. On suspected compromise, rotate immediately and note the affected ledger
   range in the release notes.

## Chain anchoring (optional)

`content_hash` is a 32-byte value, so it can be anchored by emitting it in a
contract event from an existing admin action. Anchoring is not enabled in CI;
when it is, record the transaction hash next to the snapshot.

## Workflow permissions

The workflow keeps `contents: write` (required to push `gh-pages`). The signing
key is only exposed to the single signing step and is deleted straight after.
