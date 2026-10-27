import type { Verdict } from "../../src/index.js";

export interface VerdictStyle {
  icon: string;
  label: string;
  bg: string;
  fg: string;
  positive: boolean;
}

const luminance = (hex: string): number => {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};

export const contrast = (a: string, b: string): number => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

/** Mechanical checks from docs/integration/verdict-ux-guidelines.md. Returns failures. */
export function checkVerdictStyles(styles: Record<Verdict, VerdictStyle>): string[] {
  const failures: string[] = [];
  for (const [verdict, s] of Object.entries(styles)) {
    if (!s.icon || !s.label) failures.push(`${verdict}: missing icon or label`);
    if (contrast(s.bg, s.fg) < 7) failures.push(`${verdict}: contrast below 7:1`);
    if (s.positive && verdict !== "verified") failures.push(`${verdict}: must not be positive`);
  }
  if (!styles.verified?.positive) failures.push("verified: must be positive");
  return failures;
}
