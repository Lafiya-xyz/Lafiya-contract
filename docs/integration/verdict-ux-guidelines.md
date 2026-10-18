# Responder UX Guidelines for Verification Verdicts

Tracks [#607](https://github.com/Lafiya-xyz/Lafiya-contract/issues/607). Verdict semantics live
in [verification-model.md](../specs/verification-model.md); display rules in
[lafiya-web.md](lafiya-web.md). This page covers *presentation* under time pressure, bright
sun, low-end screens, and poor connectivity.

## Principles

1. **Never colour alone.** Every verdict has colour + icon + text (redundancy).
2. **Positive is rare and explicit.** Only `Verified` looks positive; everything else looks
   cautionary or neutral.
3. **Outage is not a verdict.** `Indeterminate`/`UNAVAILABLE` never looks like "not verified".
4. **Staleness is visible.** Offline or cached results show the as-of date and say "as of".
5. **First line is the decision, second line is the reason.**

## Guidelines

| Area | Rule |
| --- | --- |
| Contrast | Text and icon vs. background >= 7:1 (WCAG AAA) for the verdict banner; >= 4.5:1 elsewhere. Test in grayscale and at full sunlight brightness. |
| Redundancy | Icon (check / warning / cross / question) + one-word label + colour. Icons carry an accessible name equal to the label. |
| Size | Verdict label >= 24 sp; reason >= 16 sp; touch targets >= 48 dp. |
| Reading level | Target grade 6 or below; sentences <= 12 words; no jargon ("attester" -> "health worker"); no abbreviations in the first line. |
| Escalation wording | Caution verdicts end with an action: "Treat with caution. Ask for another record." Never "Error". Never say "safe". |
| Staleness | Show "Checked 14:02 (2 hours ago)" or "Offline copy from 3 Oct" beside the verdict, in local time with zone. |
| Motion/sound | No auto-dismissing banners; no flashing. Optional haptic/audio cue must be redundant, never the only signal. |
| Degraded | With no network and no snapshot: show `UNAVAILABLE`, offer retry, never default to a positive or negative verdict. |

## Examples

- [Verified](verdict-ux/verified.svg)
- [Caution (attester suspended)](verdict-ux/caution.svg)
- [Unavailable](verdict-ux/unavailable.svg)

## Testable checklist for integrators

Each item is mechanically checkable; the example app implements them in
[`packages/verifier-sdk/example/react-native/a11y-checklist.ts`](../../packages/verifier-sdk/example/react-native/a11y-checklist.ts).

- [ ] Every verdict view contains an icon, a text label, and a colour class.
- [ ] Verdict banner contrast ratio >= 7:1 (computed from the two colours).
- [ ] Verdict label font size >= 24 and touch targets >= 48.
- [ ] Only `verified` maps to a positive style; every other verdict does not.
- [ ] `unavailable` has a retry control and no positive/negative styling.
- [ ] Any cached/offline result renders an "as of" timestamp with time zone.
- [ ] Reason text is <= 12 words per sentence.

## Clinical review

> **Open item:** review with clinicians has not yet happened. Record reviewer role, date,
> changes requested, and rationale here before treating these guidelines as final.

| Date | Reviewer role | Outcome / rationale |
| --- | --- | --- |
| _pending_ | _pending_ | _pending_ |
