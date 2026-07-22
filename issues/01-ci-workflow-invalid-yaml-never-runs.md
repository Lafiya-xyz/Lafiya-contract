---
title: "[bug]: CI workflow ci.yml is invalid YAML and has never actually run"
labels: bug, ci, priority:p0
---

> Root-cause issue. Files #02 and #03 in this batch are consequences of this
> one: neither the current compile break on `main` nor the botched PR #94
> merge could have been caught, because the workflow that should have
> caught them cannot parse.

## Description

`.github/workflows/ci.yml`'s `build-wasm` job has a malformed step list.
The final two lines are indented as though they were additional keys
nested under the *value* of the preceding `run:` step, which is not valid
YAML — a scalar step value cannot have sibling mapping keys hung off it at
a deeper indent.

```yaml
  build-wasm:
    name: Build (wasm32v1-none)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32v1-none
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --workspace --release --locked --target wasm32v1-none
        - name: Check README contract tables          # <-- invalid: '-' at
          run: python .github/scripts/check_readme_contracts.py   #     wrong indent
```

Confirmed by parsing the file with PyYAML:

```
$ python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"
yaml.parser.ParserError: while parsing a block mapping
  in ".github/workflows/ci.yml", line 51, column 9
expected <block end>, but found '-'
  in ".github/workflows/ci.yml", line 52, column 9
```

GitHub Actions rejects a workflow file that fails to parse — none of its
jobs (`fmt`, `clippy`, `test`, `build-wasm`) run, on either `push` to
`main` or `pull_request`. Practically, this repository has had **no
working CI** since this job was introduced, despite `CONTRIBUTING.md`
telling contributors `make check` "is the same set of checks CI runs" and
the PR template presumably gating merge on green checks.

This is the root cause that let two other confirmed problems reach `main`
undetected:
- `main` currently fails to compile (see the "main does not compile"
  issue) — `cargo test`/`cargo build` would have caught every one of
  those errors on the very next `push`.
- PR #94 ("batch add/remove attesters") merged with its actual `lib.rs`/
  `test.rs`/`docs/error-codes.md` changes silently dropped, leaving only
  the unrelated `Cargo.lock`/`.cargo/config.toml` changes — `cargo test`
  would have caught this too, since the batch-attester tests that PR
  claims to add don't exist on `main` at all (see the PR #94 issue).

## Steps to reproduce

1. `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`
2. Observe `ParserError`.
3. Alternatively: check the Actions tab for this repository for any
   `CI` workflow run on recent commits to `main` (e.g. `245d0f9`,
   `ba5f500`) and confirm none exist / none show job results for `fmt`,
   `clippy`, `test`, `build-wasm`.

## Expected behavior

`ci.yml` parses and its four jobs run on every push to `main` and on
every pull request, blocking merge on failure.

## Actual behavior

The workflow file is invalid YAML; GitHub Actions cannot schedule any of
its jobs.

## Proposed fix

Split the malformed step into two separate, correctly indented list
items:

```yaml
      - run: cargo build --workspace --release --locked --target wasm32v1-none
      - name: Check README contract tables
        run: python .github/scripts/check_readme_contracts.py
```

Then, as part of this issue's acceptance criteria, confirm the fix by
actually observing a green (or intentionally red, to prove signal) run in
the Actions tab — not just local YAML validation — since this exact class
of failure is invisible without checking.

## Acceptance criteria

- [ ] `ci.yml` parses as valid YAML (`yamllint` or `actionlint` in a
      pre-commit hook or a meta-CI check would prevent recurrence —
      consider adding `actionlint` to `pre-commit` config).
- [ ] A workflow run is visible in the Actions tab for a push to a branch
      touching this file, showing all four jobs (`fmt`, `clippy`, `test`,
      `build-wasm`) actually executing.
- [ ] Branch protection on `main` is confirmed to require these checks
      (currently unverifiable from the repo alone — needs a maintainer
      with repo-settings access to confirm required status checks are
      configured, since a non-required check would have let broken code
      merge even with a passing workflow file).

## Environment

- Contract(s) affected: CI tooling (`.github/workflows/ci.yml`)
- This is a pure configuration bug, not a `rustc`/`soroban-sdk` issue.
