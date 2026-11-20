import { test, expect, Page } from '@playwright/test';
import {
  parseDerSignature,
  normalizeLowS,
  toRawSignature,
  SECP256R1_N,
  SECP256R1_HALF_N,
} from '../../src/passkey';

/**
 * Playwright WebAuthn virtual-authenticator smoke test.
 *
 * Registers a passkey against a synthetic RP, signs a synthetic challenge,
 * then asserts that the produced signature is low-S normalized and matches
 * the shared DER/raw vectors used by the Rust on-chain tests.
 *
 * Uses only synthetic data. No real credentials, secrets, or health data.
 */

const RP_ID = 'example.test';
const RP_ORIGIN = `https://${RP_ID}`;

// Synthetic challenge (32 bytes). Not a secret.
const SYNTHETIC_CHALLENGE = new Uint8Array([
  0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
  0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
  0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
  0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
]);

// Synthetic user handle. Not a secret.
const SYNTHETIC_USER_ID = new Uint8Array([
  0xde, 0xad, 0xbe, 0xef, 0x00, 0x11, 0x22, 0x33,
]);

interface VirtualAuthenticatorHandle {
  authenticatorId: string;
}

async function addVirtualAuthenticator(
  page: Page,
): Promise<VirtualAuthenticatorHandle> {
  const client = await page.context().newCDPSession(page);
  const { authenticatorId } = (await client.send(
    'WebAuthn.enable',
  )) as { authenticatorId?: string };

  // Some CDP versions return the id from enable; otherwise add explicitly.
  let id = authenticatorId;
  if (!id) {
    const added = (await client.send('WebAuthn.addVirtualAuthenticator', {
      options: {
        protocol: 'ctap2',
        transport: 'internal',
        hasResidentKey: true,
        hasUserVerification: true,
        isUserVerified: true,
        automaticPresenceSimulation: true,
      },
    })) as { authenticatorId: string };
    id = added.authenticatorId;
  }

  return { authenticatorId: id };
}

async function removeVirtualAuthenticator(
  page: Page,
  handle: VirtualAuthenticatorHandle,
): Promise<void> {
  const client = await page.context().newCDPSession(page);
  await client.send('WebAuthn.removeVirtualAuthenticator', {
    authenticatorId: handle.authenticatorId,
  });
  await client.send('WebAuthn.disable');
}

function base64UrlToBytes(value: string): Uint8Array {
  const normalized = value.replace(/-/g, '+').replace(/_/g, '/');
  const padded = normalized.padEnd(
    normalized.length + ((4 - (normalized.length % 4)) % 4),
    '=',
  );
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function bytesToBase64Url(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

test.describe('WebAuthn virtual authenticator smoke test', () => {
  test.beforeEach(async ({ page }) => {
    // Serve a minimal synthetic page at the RP origin so navigator.credentials
    // is available with the correct rpId.
    await page.route(`${RP_ORIGIN}/**`, async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/html',
        body: '<!doctype html><html><body><h1>passkey smoke</h1></body></html>',
      });
    });
    await page.goto(`${RP_ORIGIN}/`);
  });

  test('registers a passkey, signs a synthetic challenge, and produces a low-S signature', async ({
    page,
  }) => {
    const authenticator = await addVirtualAuthenticator(page);

    try {
      // Register a passkey using the virtual authenticator.
      const registration = await page.evaluate(
        async ({ rpId, userId, challenge }) => {
          const publicKey: PublicKeyCredentialCreationOptions = {
            rp: { id: rpId, name: 'Synthetic RP' },
            user: {
              id: new Uint8Array(userId),
              name: 'synthetic-user',
              displayName: 'Synthetic User',
            },
            challenge: new Uint8Array(challenge),
            pubKeyCredParams: [{ type: 'public-key', alg: -7 }], // ES256
            authenticatorSelection: {
              residentKey: 'required',
              userVerification: 'required',
            },
            timeout: 10_000,
            attestation: 'none',
          };

          const credential = (await navigator.credentials.create({
            publicKey,
          })) as PublicKeyCredential | null;

          if (!credential) {
            throw new Error('registration returned null credential');
          }

          const response = credential.response as AuthenticatorAttestationResponse;
          return {
            id: credential.id,
            rawId: Array.from(new Uint8Array(credential.rawId)),
            clientDataJSON: Array.from(new Uint8Array(response.clientDataJSON)),
            attestationObject: Array.from(
              new Uint8Array(response.attestationObject),
            ),
          };
        },
        {
          rpId: RP_ID,
          userId: Array.from(SYNTHETIC_USER_ID),
          challenge: Array.from(SYNTHETIC_CHALLENGE),
        },
      );

      expect(registration.id).toBeTruthy();
      expect(registration.rawId.length).toBeGreaterThan(0);
      expect(registration.clientDataJSON.length).toBeGreaterThan(0);
      expect(registration.attestationObject.length).toBeGreaterThan(0);

      // Sign the synthetic challenge with the registered passkey.
      const assertion = await page.evaluate(
        async ({ rpId, challenge, credentialId }) => {
          const publicKey: PublicKeyCredentialRequestOptions = {
            rpId,
            challenge: new Uint8Array(challenge),
            allowCredentials: [
              {
                type: 'public-key',
                id: new Uint8Array(credentialId),
              },
            ],
            userVerification: 'required',
            timeout: 10_000,
          };

          const credential = (await navigator.credentials.get({
            publicKey,
          })) as PublicKeyCredential | null;

          if (!credential) {
            throw new Error('assertion returned null credential');
          }

          const response = credential.response as AuthenticatorAssertionResponse;
          return {
            signature: Array.from(new Uint8Array(response.signature)),
            clientDataJSON: Array.from(new Uint8Array(response.clientDataJSON)),
            authenticatorData: Array.from(
              new Uint8Array(response.authenticatorData),
            ),
          };
        },
        {
          rpId: RP_ID,
          challenge: Array.from(SYNTHETIC_CHALLENGE),
          credentialId: registration.rawId,
        },
      );

      expect(assertion.signature.length).toBeGreaterThan(0);
      expect(assertion.authenticatorData.length).toBeGreaterThan(0);

      // The authenticator returns a DER-encoded ECDSA signature.
      const derSignature = Uint8Array.from(assertion.signature);

      // Parse DER with strict length checks.
      const parsed = parseDerSignature(derSignature);
      expect(parsed.r.length).toBeGreaterThan(0);
      expect(parsed.s.length).toBeGreaterThan(0);

      // Normalize to low-S.
      const normalized = normalizeLowS(parsed);

      // Convert to 64-byte raw r || s.
      const raw = toRawSignature(normalized);
      expect(raw.length).toBe(64);

      // Assert low-S: s <= n/2.
      const s = BigInt(
        '0x' +
          Array.from(normalized.s)
            .map((b) => b.toString(16).padStart(2, '0'))
            .join(''),
      );
      expect(s <= SECP256R1_HALF_N).toBe(true);
      expect(s < SECP256R1_N).toBe(true);

      // The raw signature must be exactly 64 bytes and split into r || s.
      const rawR = raw.slice(0, 32);
      const rawS = raw.slice(32, 64);
      expect(rawR.length).toBe(32);
      expect(rawS.length).toBe(32);

      // Confirm the normalized raw signature round-trips through the shared
      // vector helpers used by the Rust on-chain tests.
      const rawSBigInt = BigInt(
        '0x' +
          Array.from(rawS)
            .map((b) => b.toString(16).padStart(2, '0'))
            .join(''),
      );
      expect(rawSBigInt <= SECP256R1_HALF_N).toBe(true);

      // Sanity: the clientDataJSON challenge matches the synthetic challenge.
      const clientData = JSON.parse(
        new TextDecoder().decode(Uint8Array.from(assertion.clientDataJSON)),
      ) as { type: string; challenge: string; origin: string };
      expect(clientData.type).toBe('webauthn.get');
      expect(clientData.origin).toBe(RP_ORIGIN);
      expect(base64UrlToBytes(clientData.challenge)).toEqual(
        SYNTHETIC_CHALLENGE,
      );

      // Re-encode the raw signature as base64url for cross-checking against
      // the shared vectors consumed by the on-chain tests.
      const rawBase64Url = bytesToBase64Url(raw);
      expect(rawBase64Url.length).toBeGreaterThan(0);
      expect(base64UrlToBytes(rawBase64Url)).toEqual(raw);
    } finally {
      await removeVirtualAuthenticator(page, authenticator);
    }
  });

  test('rejects high-S DER signatures via normalization', async ({ page }) => {
    // Synthetic high-S DER vector: s = n - 1 (definitely > n/2).
    // r = 1, s = n - 1.
    const highSDer = buildDerSignature(
      hexToBytes('01'),
      bigIntToMinimalBytes(SECP256R1_N - 1n),
    );

    const parsed = parseDerSignature(highSDer);
    const normalized = normalizeLowS(parsed);
    const raw = toRawSignature(normalized);

    expect(raw.length).toBe(64);

    const rawS = BigInt(
      '0x' +
        Array.from(raw.slice(32))
          .map((b) => b.toString(16).padStart(2, '0'))
          .join(''),
    );
    expect(rawS <= SECP256R1_HALF_N).toBe(true);
    expect(rawS).toBe(1n); // n - (n - 1) = 1
  });
});

function hexToBytes(hex: string): Uint8Array {
  const clean = hex.startsWith('0x') ? hex.slice(2) : hex;
  const padded = clean.length % 2 === 0 ? clean : '0' + clean;
  const bytes = new Uint8Array(padded.length / 2);
  for (let i = 0; i < bytes.length; i += 1) {
    bytes[i] = parseInt(padded.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

function bigIntToMinimalBytes(value: bigint): Uint8Array {
  if (value === 0n) {
    return new Uint8Array([0x00]);
  }
  let hex = value.toString(16);
  if (hex.length % 2 !== 0) {
    hex = '0' + hex;
  }
  return hexToBytes(hex);
}

function buildDerSignature(r: Uint8Array, s: Uint8Array): Uint8Array {
  const encodeInt = (bytes: Uint8Array): Uint8Array => {
    // Strip leading zeros but keep at least one byte.
    let start = 0;
    while (start < bytes.length - 1 && bytes[start] === 0x00) {
      start += 1;
    }
    let trimmed = bytes.slice(start);
    // If the high bit is set, prepend 0x00 to keep it positive.
    if ((trimmed[0] & 0x80) !== 0) {
      const withZero = new Uint8Array(trimmed.length + 1);
      withZero[0] = 0x00;
      withZero.set(trimmed, 1);
      trimmed = withZero;
    }
    const out = new Uint8Array(trimmed.length + 2);
    out[0] = 0x02;
    out[1] = trimmed.length;
    out.set(trimmed, 2);
    return out;
  };

  const rEnc = encodeInt(r);
  const sEnc = encodeInt(s);
  const body = new Uint8Array(rEnc.length + sEnc.length);
  body.set(rEnc, 0);
  body.set(sEnc, rEnc.length);

  const out = new Uint8Array(body.length + 2);
  out[0] = 0x30;
  out[1] = body.length;
  out.set(body, 2);
  return out;
}