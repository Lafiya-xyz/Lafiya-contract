/**
 * Byte helpers with no Node built-ins, so the SDK core runs unchanged on
 * browsers, Node, React Native (Hermes) and Expo.
 */

/** Decode a lowercase or uppercase hex string into bytes. */
export function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error("hex string must have an even length");
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    const byte = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    if (Number.isNaN(byte)) throw new Error("hex string contains a non-hex character");
    bytes[i] = byte;
  }
  return bytes;
}

/** Encode bytes as a lowercase hex string. */
export function bytesToHex(bytes: Uint8Array): string {
  let out = "";
  for (const byte of bytes) out += byte.toString(16).padStart(2, "0");
  return out;
}
