# Lafiya reference mobile verifier (React Native)

Reference for the field verification flow (#603): scan QR, evaluate with `@lafiya/verifier`,
show verdict with tier and staleness, fall back to a cached offline snapshot.

- `App.tsx`: scan, verify, offline fallback, background snapshot refresh.
- `styles.ts` / `a11y-checklist.ts`: verdict styling and the mechanical checks from
  [verdict-ux-guidelines](../../../../docs/integration/verdict-ux-guidelines.md).
- `trust.ts`: trust anchors (fill from the release manifest).

No analytics or crash reporters are included; the only network traffic is the configured
Soroban RPC / indexer.

## Status

This is a scaffold. Not yet done, and required before this issue's acceptance criteria are met:

- [ ] Wire into a generated RN project (`npx @react-native-community/cli init`) and pin deps
      (`react-native-vision-camera`, `@react-native-async-storage/async-storage`).
- [ ] Automated UI tests with scripted QR fixtures (e.g. Maestro/Detox; fixtures from
      `encodeQrPayload`).
- [ ] Measure on the low-end Android reference device and record budgets here.
- [ ] Accessibility run (TalkBack) on verdict screens.
