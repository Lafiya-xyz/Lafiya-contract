// Reference verifier: scan QR -> LafiyaVerifier -> verdict with tier and staleness.
// Network use is limited to the Soroban RPC / indexer configured in TRUST.
import React, { useCallback, useEffect, useState } from "react";
import { AccessibilityInfo, Pressable, SafeAreaView, Text, View } from "react-native";
import { Camera, useCameraDevice, useCodeScanner } from "react-native-vision-camera";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { LafiyaVerifier, OfflineChainReader, VERDICT_DESCRIPTIONS, type OfflineBundle, type VerificationResult } from "@lafiya/verifier";
import { VERDICT_STYLES } from "./styles";
import { TRUST } from "./trust";

const BUNDLE_KEY = "lafiya.offline-bundle";
const online = new LafiyaVerifier({ network: "testnet", trust: TRUST });

/** Refresh the cached snapshot in the background; failures leave the old snapshot. */
async function refreshSnapshot(fetchBundle: () => Promise<OfflineBundle>) {
  try {
    await AsyncStorage.setItem(BUNDLE_KEY, JSON.stringify(await fetchBundle()));
  } catch {
    /* offline: keep the previous snapshot */
  }
}

async function verify(text: string): Promise<VerificationResult> {
  const result = await online.verifyCard(text, { maxAgeDays: 365 });
  if (result.verdict !== "unavailable") return result;
  const cached = await AsyncStorage.getItem(BUNDLE_KEY);
  if (!cached) return result;
  const offline = new LafiyaVerifier({ network: "testnet", trust: TRUST, chain: new OfflineChainReader(JSON.parse(cached)) });
  return offline.verifyCard(text, { maxAgeDays: 365 });
}

export default function App() {
  const device = useCameraDevice("back");
  const [result, setResult] = useState<VerificationResult | null>(null);
  const scanner = useCodeScanner({
    codeTypes: ["qr"],
    onCodeScanned: async ([code]) => {
      if (code?.value) setResult(await verify(code.value));
    },
  });
  useEffect(() => {
    if (result) AccessibilityInfo.announceForAccessibility(VERDICT_DESCRIPTIONS[result.verdict]);
  }, [result]);
  const reset = useCallback(() => setResult(null), []);

  if (!result) {
    return <SafeAreaView style={{ flex: 1 }}>{device && <Camera style={{ flex: 1 }} device={device} isActive codeScanner={scanner} />}</SafeAreaView>;
  }
  const s = VERDICT_STYLES[result.verdict];
  const offline = result.source === "offline";
  return (
    <SafeAreaView style={{ flex: 1, backgroundColor: s.bg, padding: 24 }} accessibilityLiveRegion="assertive">
      <Text style={{ color: s.fg, fontSize: 64 }} accessibilityElementsHidden importantForAccessibility="no">{s.icon}</Text>
      <Text accessibilityRole="header" style={{ color: s.fg, fontSize: 32, fontWeight: "700" }}>{s.label}</Text>
      <Text style={{ color: s.fg, fontSize: 18 }}>{VERDICT_DESCRIPTIONS[result.verdict]}</Text>
      {result.attester && <Text style={{ color: s.fg, fontSize: 16 }}>Health worker: {result.attester.status}</Text>}
      <Text style={{ color: s.fg, fontSize: 16 }}>
        {offline ? "Offline copy as of " : "Checked "}
        {result.checkedAt.toLocaleString()}
      </Text>
      <View style={{ flex: 1 }} />
      <Pressable accessibilityRole="button" accessibilityLabel="Scan another card" onPress={reset} style={{ minHeight: 48, justifyContent: "center" }}>
        <Text style={{ color: s.fg, fontSize: 20 }}>{result.verdict === "unavailable" ? "Retry" : "Scan another card"}</Text>
      </Pressable>
    </SafeAreaView>
  );
}
