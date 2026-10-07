/**
 * Passkey ceremonies from the client's side.
 *
 * The server speaks WebAuthn options and credentials as JSON with every binary field in
 * base64url; the browser API wants and returns `ArrayBuffer`s. The conversions here are the
 * same ones the server's own handoff page (`server/api/src/passkey_page/page.html`) performs.
 *
 * A page can run a ceremony itself only when its host is the relying party's domain or under
 * it (`canRunInPage`). The desktop and mobile shells cannot, so they hand the ceremony to that
 * page in the system browser through a `PasskeyHandoff`, which the shell implements, and claim
 * the result with a PKCE secret (RFC 7636), proving they started it, and the return code the
 * browser brought back, proving the ceremony ran on this device.
 */

import type { components } from "./generated/openapi";

type Schemas = components["schemas"];

export type PasskeyPurpose = Schemas["PasskeyPurpose"];
export type Passkey = Schemas["Passkey"];

/** How the browser came back to the app from the handoff page. */
export interface HandoffReturn {
  ceremony: string;
  outcome: "done" | "cancelled";
  /**
   * The return code a finished ceremony brought back, which the claim presents along with the
   * PKCE secret; `null` when cancelled. It reaches only the return address, so a ceremony whose
   * page was opened on someone else's device is never claimed.
   */
  code: string | null;
}

/** One handoff in progress: a return address listening for the browser. */
export interface HandoffSession {
  /** Where the page sends the browser when it is done. */
  returnTo: string;
  /** Opens the handoff page in the system browser and resolves when the browser returns. */
  open(url: string): Promise<HandoffReturn>;
  /** Stops listening; called however the ceremony ends. */
  dispose(): void;
}

/** What a shell provides to run ceremonies in the system browser. */
export interface PasskeyHandoff {
  prepare(): Promise<HandoffSession>;
}

/** How a ceremony reaches an authenticator. */
export type PasskeyTransport = { kind: "inPage" } | { kind: "handoff"; handoff: PasskeyHandoff };

/** The user dismissed the passkey prompt or the handoff page. */
export class PasskeyCancelledError extends Error {
  constructor() {
    super("passkey ceremony cancelled");
    this.name = "PasskeyCancelledError";
  }
}

/** Whether a page at `hostname` can run ceremonies for `rpId` itself. */
export function canRunInPage(rpId: string, hostname: string, hasWebAuthn: boolean): boolean {
  return hasWebAuthn && (hostname === rpId || hostname.endsWith(`.${rpId}`));
}

export function fromBase64url(text: string): Uint8Array<ArrayBuffer> {
  const base64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(base64 + "===".slice((base64.length + 3) % 4));
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}

export function toBase64url(buffer: ArrayBuffer | Uint8Array): string {
  let binary = "";
  for (const byte of new Uint8Array(buffer)) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

interface JsonDescriptor {
  id: string;
  type: "public-key";
  transports?: AuthenticatorTransport[];
}

interface JsonPublicKey extends Record<string, unknown> {
  challenge: string;
  user?: { id: string; name: string; displayName: string };
  excludeCredentials?: JsonDescriptor[];
  allowCredentials?: JsonDescriptor[];
}

function publicKeyOf(options: unknown): JsonPublicKey {
  const publicKey = (options as { publicKey?: JsonPublicKey } | null)?.publicKey;
  if (publicKey === undefined || typeof publicKey.challenge !== "string") {
    throw new Error("passkey options carry no publicKey");
  }
  return publicKey;
}

function decodeDescriptors(list: JsonDescriptor[] | undefined) {
  return list?.map((descriptor) => ({ ...descriptor, id: fromBase64url(descriptor.id) }));
}

/** The server's registration options, ready for `navigator.credentials.create`. */
export function creationOptions(options: unknown): PublicKeyCredentialCreationOptions {
  const publicKey = publicKeyOf(options);
  const user = publicKey.user;
  if (user === undefined) {
    throw new Error("registration options carry no user");
  }
  const excludeCredentials = decodeDescriptors(publicKey.excludeCredentials);
  return {
    ...(publicKey as unknown as PublicKeyCredentialCreationOptions),
    challenge: fromBase64url(publicKey.challenge),
    user: { ...user, id: fromBase64url(user.id) },
    ...(excludeCredentials === undefined ? {} : { excludeCredentials }),
  };
}

/** The server's authentication options, ready for `navigator.credentials.get`. */
export function requestOptions(options: unknown): PublicKeyCredentialRequestOptions {
  const publicKey = publicKeyOf(options);
  const allowCredentials = decodeDescriptors(publicKey.allowCredentials);
  return {
    ...(publicKey as unknown as PublicKeyCredentialRequestOptions),
    challenge: fromBase64url(publicKey.challenge),
    ...(allowCredentials === undefined ? {} : { allowCredentials }),
  };
}

/** A credential the browser returned, as the JSON the server reads. */
export function encodeCredential(credential: PublicKeyCredential): Record<string, unknown> {
  const response = credential.response;
  const encoded: Record<string, unknown> & { response: Record<string, unknown> } = {
    id: credential.id,
    rawId: toBase64url(credential.rawId),
    type: credential.type,
    clientExtensionResults: credential.getClientExtensionResults(),
    response: { clientDataJSON: toBase64url(response.clientDataJSON) },
  };
  if ("attestationObject" in response) {
    const attestation = response as AuthenticatorAttestationResponse;
    encoded.response.attestationObject = toBase64url(attestation.attestationObject);
    encoded.response.transports = attestation.getTransports();
  } else {
    const assertion = response as AuthenticatorAssertionResponse;
    encoded.response.authenticatorData = toBase64url(assertion.authenticatorData);
    encoded.response.signature = toBase64url(assertion.signature);
    if (assertion.userHandle !== null) {
      encoded.response.userHandle = toBase64url(assertion.userHandle);
    }
  }
  return encoded;
}

/** Runs a ceremony against the browser's own authenticators. */
export async function runInPage(
  purpose: PasskeyPurpose,
  options: unknown,
): Promise<Record<string, unknown>> {
  let credential: Credential | null;
  try {
    credential =
      purpose === "register"
        ? await navigator.credentials.create({ publicKey: creationOptions(options) })
        : await navigator.credentials.get({ publicKey: requestOptions(options) });
  } catch (e) {
    // The browser reports a dismissed prompt, a timeout, and "no credential here" alike.
    if (e instanceof DOMException && e.name === "NotAllowedError") {
      throw new PasskeyCancelledError();
    }
    throw e;
  }
  if (credential === null) {
    throw new PasskeyCancelledError();
  }
  return encodeCredential(credential as PublicKeyCredential);
}

/** A PKCE verifier and its `S256` challenge. */
export async function pkcePair(): Promise<{ verifier: string; challenge: string }> {
  const verifier = toBase64url(crypto.getRandomValues(new Uint8Array(32)));
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier));
  return { verifier, challenge: toBase64url(digest) };
}

/** The address of the server's handoff page for one ceremony. */
export function handoffPageUrl(baseUrl: string, ceremony: string): string {
  return `${baseUrl}/auth/passkey#ceremony=${encodeURIComponent(ceremony)}`;
}

/**
 * Reads the browser's return from a return address: the `ceremony`, `outcome`, and `code` the
 * handoff page added to its query. `null` when the URL is not such a return.
 */
export function parseHandoffReturn(url: string): HandoffReturn | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const ceremony = parsed.searchParams.get("ceremony");
  const outcome = parsed.searchParams.get("outcome");
  if (ceremony === null || (outcome !== "done" && outcome !== "cancelled")) {
    return null;
  }
  return { ceremony, outcome, code: parsed.searchParams.get("code") };
}
