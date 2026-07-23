---
title: "[audit] CI-01: ci.yml is malformed YAML — no CI job has ever executed"
labels: bug, ci, priority:p0, severity:critical
---

**Severity:** Critical
**Difficulty:** Trivial (always active; no attacker action required)
**Type:** CI/CD Configuration Integrity — Control Bypass

> Root-cause finding. BUILD-01 and PROC-01 are direct consequences:
> neither the current compile break on `main` nor the incomplete PR #94
> merge could have been caught, because the workflow that should have
> caught them does not parse.

## Summary

`.github/workflows/ci.yml` is invalid YAML. GitHub Actions cannot
schedule any job defined in it. Every quality gate this repository
believes it has — `fmt`, `clippy`, `test`, `build-wasm` — has not run on
a single push or pull request since the defect was introduced. This is
the mechanism that allowed CI-02 (broken build on `main`) and CI-03 (a
merged PR missing its own diff) to land without detection.

## Location

`.github/workflows/ci.yml:42-53` (`build-wasm` job)

## Technical Detail

```yaml
42	  build-wasm:
43	    name: Build (wasm32v1-none)
44	    runs-on: ubuntu-latest
45	    steps:
46	      - uses: actions/checkout@v4
47	      - uses: dtolnay/rust-toolchain@stable
48	        with:
49	          targets: wasm32v1-none
50	      - uses: Swatinem/rust-cache@v2
51	      - run: cargo build --workspace --release --locked --target wasm32v1-none
52	        - name: Check README contract tables
53	          run: python .github/scripts/check_readme_contracts.py
```

Lines 52-53 are indented as additional mapping keys nested under the
scalar value of the `run:` key on line 51. A YAML block-scalar step
cannot have sibling keys hung off it at a deeper indent — this is a
parser error, not a style issue.

## Proof of Concept

```
$ python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"
Traceback (most recent call last):
  ...
yaml.parser.ParserError: while parsing a block mapping
  in ".github/workflows/ci.yml", line 51, column 9
expected <block end>, but found '-'
  in ".github/workflows/ci.yml", line 52, column 9
```

A workflow file that fails to parse is rejected by GitHub Actions in its
entirety — not just the malformed job. `fmt`, `clippy`, and `test`, which
are syntactically valid on their own, do not run either, because they
live in the same file.

## Impact

No merge to `main`, and no pull request, has been gated by `cargo fmt
--check`, `cargo clippy -D warnings`, `cargo test --workspace`, or `cargo
build --release --target wasm32v1-none` for as long as this defect has
existed. `CONTRIBUTING.md` states `make check` "is the same set of checks
CI runs" and the PR process implies checks gate merge — neither has been
true in practice. This is the direct enabling condition for:
- BUILD-01 — `main` currently fails `cargo build`/`cargo test` outright.
- PROC-01 — PR #94 merged with its stated `lib.rs`/`test.rs`/docs changes
  silently absent; the 12 tests that PR claims to add have never run.

## Recommendation

Split the malformed step into two correctly indented list items:

```yaml
      - run: cargo build --workspace --release --locked --target wasm32v1-none
      - name: Check README contract tables
        run: python .github/scripts/check_readme_contracts.py
```

Add `actionlint` (or `yamllint`) to the `pre-commit` configuration
referenced in `CONTRIBUTING.md` so a workflow-file syntax regression is
caught locally before push, not discovered by audit.

## Verification

- [ ] `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"` exits cleanly.
- [ ] A workflow run appears in the Actions tab for a push touching this
      file, showing all four jobs (`fmt`, `clippy`, `test`, `build-wasm`)
      actually executing — local YAML validation alone does not prove
      GitHub schedules the jobs.
- [ ] Branch protection on `main` is confirmed (by a maintainer with
      repo-settings access) to list these jobs as *required* status
      checks — a passing-but-optional check would not have prevented
      CI-02/CI-03 either.
