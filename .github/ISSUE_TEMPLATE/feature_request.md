---
name: Feature request
about: Suggest an idea or improvement for the contracts or tooling.
title: "[feature]: "
labels: enhancement
---

> **Security-related proposal?** If describing it publicly would reveal a
> vulnerability, don't file an issue — use the private process in
> [SECURITY.md](SECURITY.md) instead.

## Problem

What problem does this solve, and for whom? (e.g. attesters, responders,
CHWs, integrators).

## Proposed change

What you would like to see added or changed, and roughly how.

## Alternatives considered

Other approaches you thought about, and why they fit less well.

## Scope

- Component(s) affected: <!-- attester-registry / attestation-registry / CI / docs -->
- Does this change a contract's public function signatures or the
  attestation schema (`record hash + attester + timestamp`)? If yes, say
  so explicitly — that schema is a cross-repo contract consumed by
  `lafiya-web` (see the README's
  [Shared Contracts](README.md#shared-contracts-must-stay-in-sync-across-repos)
  section) and the change must be called out so the web app can be
  updated in step.

## Additional context

Links, prior art, related issues. The contribution workflow (`make
check`, test expectations) is in [CONTRIBUTING.md](CONTRIBUTING.md).
