# AI.md — Lafiya Organization Map

Context for an AI agent working in any single repo under the `lafiya-xyz`
GitHub organization: what the other repos are, how they fit together, and
what must stay in sync if you change something here.

## Repos

| Repo | URL | Purpose | Priority |
|------|-----|---------|----------|
| `lafiya-web` | https://github.com/Lafiya-xyz/lafiya-web | Patient + responder web app (Next.js). Public emergency page, authed profile editor, QR generation. | Build first |
| `lafiya-contracts` | https://github.com/Lafiya-xyz/Lafiya-contract | Soroban smart contracts (Rust): attestation registry + attester allowlist. Testnet first. | Build next |
| `lafiya-docs` | https://github.com/Lafiya-xyz/lafiya-docs | Concept note, data model, threat model, privacy design, funding/DPG materials, references. | Start now (lightweight) |
| `.github` | https://github.com/Lafiya-xyz/.github | Organization profile README and contribution guidelines. | Start now |
| `lafiya-verifier` | https://github.com/Lafiya-xyz/lafiya-verifier | CHW verification tool. Begins as a route inside `lafiya-web`; split out only if it grows. | Later |

## Data flow

```
lafiya-web  ──(record hash)──▶  lafiya-contracts
                                       │
        CHW attests ──(licensed?)──▶  │  (attester allowlist check)
                                       ▼
                          attestation: hash + attester id + timestamp
                                       │
                                       ▼
                              lafiya-web public emergency page
                                       │
                                       ▼
                         responder scans QR, sees verified indicator
```

## Shared contracts (must stay in sync across repos)

**Attestation schema** — a hash of the record + the attester's identity +
a timestamp, defined by `lafiya-contracts` and consumed by `lafiya-web`'s
public emergency page. If the shape of an attestation changes in
`lafiya-contracts`, `lafiya-web`'s verification-display logic must be
updated in the same change set (or a tracked follow-up opened there).

## Conventions for AI agents

- Treat this file as the source of truth for **cross-repo** context. Each
  repo's own README covers repo-local conventions (build/test commands,
  file layout, in-progress work).
- When a change in one repo affects a shared contract above, call it out
  explicitly so the corresponding change can be made in the other repo(s).
- Keep field names identical (same casing, same units) for shared data
  across Rust (`lafiya-contracts`) and TypeScript (`lafiya-web`) —
  translation layers are a common source of bugs.
- No personal health data belongs on-chain, in any repo, ever — only
  hashes, attestations, and payments. This is a hard invariant of the
  whole system, not a per-repo style choice.
