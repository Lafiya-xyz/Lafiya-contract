# Public status page

`scripts/status_probe.py` runs public-read-path probes per deployment: RPC
liveness (`getHealth`), indexer lag (`/health`), and verification of a known
fixture attestation through the indexer. (A direct contract view call is
covered by the indexer fixture lookup, which reads contract-derived events;
add a `stellar contract invoke` probe here if a direct call is required.)
No secrets are used and nothing is written on-chain.

`scripts/status_page.py` renders a static `index.html` + `status.json` from the
probe histories. A probe is DOWN only if it fails from **all** locations in a
5-minute bucket, which suppresses single-vantage false positives. History is
retained for `--retention-days` (default 90); incidents are annotated in
`incidents.json` (`start`, `end`, `title`, `note`).

The `Status probes` workflow runs the probes every 5 minutes from two
independent GitHub-hosted regions' matrix jobs (see Open items) and
publishes history on the `gh-pages` branch. The false-positive rate must be
measured from a week of `status.json` data before the page is announced.

## Open items
- GitHub-hosted runners do not offer region pinning; the second location needs
  a runner/cron on a different provider that calls the same script and pushes
  its own `history-<location>.jsonl`.
- Repository variables required: `LAFIYA_STATUS_DEPLOYMENT`, `LAFIYA_RPC_URL`,
  `LAFIYA_INDEXER_URL`, `LAFIYA_FIXTURE_HASH`.
