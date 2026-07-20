## Description
<!-- Provide a clear summary of the changes and the motivation/context behind them. -->

## Type of Change
- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Documentation update

## Quality Checklist
- [ ] Code follows the style guidelines of this project
- [ ] I have run `make check` locally and it passes with no warnings or errors
- [ ] I have added unit tests covering success, failure, and authorization cases
- [ ] I have commented my code, particularly in hard-to-understand areas
- [ ] I have updated the documentation / README if applicable

## Cross-Repo & Shared-Contract Impact
- [ ] **Does this PR change a shared contract interface?** (e.g., attestation schema, public contract function signatures, events emitted)
  - If yes, please describe the impact and link the corresponding issue/PR in [lafiya-web](https://github.com/Lafiya-xyz/lafiya-web):
  - *Impact detail:* 

- [ ] **Does this PR require a database/Supabase migration?**
  - If yes, verify that:
    - [ ] The migration SQL is written in `lafiya-web`.
    - [ ] Hand-authored types in `lafiya-web/lib/supabase/types.ts` have been updated manually.
