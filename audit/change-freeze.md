# Change-Freeze Procedure

## Starting the freeze

1. Complete the [fresh-eyes review](#sign-off) of this package.
2. Tag the freeze commit: `git tag audit-<auditor>-<yyyy-mm>-freeze <sha>`
   and push the tag.
3. Record the commit in [scope.md](scope.md#commit) and regenerate the
   line counts there.
4. Run `make audit-env` on the tag and attach `target/audit/` to the
   audit kickoff.
5. From this point, PRs that touch `contracts/` need the
   `audit-freeze-exception` label and approval from the audit owner.
   Documentation and off-chain tooling can still change.

## Fixes during the audit

1. Each auditor finding gets an ID from the auditor (for example
   `AUD-01`). Create a private GitHub Security Advisory for Medium and
   above, or a public issue labelled `audit` for Low and informational
   findings.
2. Fix each finding in its own PR (in the advisory's private fork for
   private findings). The PR title starts with the finding ID. Add a
   regression test and update [invariants.md](invariants.md).
3. Keep a table of findings in the audit tracking issue:

   | Finding | Severity | Status | Fix PR | Fix commit | Auditor verified |
   |---|---|---|---|---|---|

4. Ask the auditors to review each fix commit. The audit report must
   list the commit that each finding was verified against.

## Closing the freeze

1. The final audited commit is the commit the auditors verified all fixes
   against. Tag it `audit-<auditor>-<yyyy-mm>-final`.
2. Record the final commit in [scope.md](scope.md#commit) and in the
   "Audits" table of [SECURITY.md](../SECURITY.md#audits), with a link to
   the published report.
3. Build the Wasm from that tag. The release manifest
   ([ADR 0010](../docs/adr/0010-release-manifest-and-compatibility.md))
   for the mainnet release must record the audited commit and the Wasm
   SHA-256 values, which must match the hashes in the final
   `target/audit/wasm-sha256.txt`.
4. A deployment is "audited" only if its on-chain Wasm hash matches one
   of these hashes. Any later contract change needs a new audit or a
   documented, auditor-reviewed diff.

## Sign-off

| Date | Reviewer (did not write the package) | Commit | Notes |
|---|---|---|---|
| _pending_ | | | |
