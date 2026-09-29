/**
 * Waking the phone when Aspen is not open (`spec/push.md`): registering it with its relay,
 * subscribing it to each deployment it is signed in to, and reading a push when one arrives.
 *
 * What the phone's own notification code needs while the app is closed (the relay subscription
 * each push names, the account it is for, the keys that decrypt it, and a session to fetch
 * with) is one JSON `PushState`, kept where that code can read it (`PushBridge`).
 */

import type { AspenClient } from "./http";

/** Which push service the phone is reached through, and as which app. */
export interface PushDevice {
  readonly platform: "apns" | "fcm";
  /** The iOS bundle id, or the Firebase project. */
  readonly app: string;
  readonly environment: "production" | "sandbox";
  /** The token the platform gave the app. */
  readonly token: string;
}

/**
 * A phone's keys for one subscription (RFC 8291): its P-256 key pair, the private half as a JWK,
 * and its 16-byte authentication secret. `publicKey` and `auth` are base64url.
 */
export interface PushKeys {
  readonly privateKey: JsonWebKey;
  readonly publicKey: string;
  readonly auth: string;
}

/** One account the phone is woken for. */
export interface PushAccount {
  /** The relay's id for the subscription, which each push names. */
  readonly subscription: string;
  /** The deployment's API origin, and the account there. */
  readonly origin: string;
  readonly userId: string;
  /** The session to fetch with; the refresh token makes new session tokens. */
  readonly refreshToken: string;
  readonly sessionToken: string;
  readonly keys: PushKeys;
  /** The deployment key the subscription was made for; a new one means subscribing again. */
  readonly applicationServerKey: string;
  /** The deployment's id for the subscription, to delete it with. */
  readonly deploymentSubscription: string;
}

/** Everything the phone keeps for push, as the native notification code reads it. */
export interface PushState {
  readonly version: 1;
  readonly device: { readonly id: string; readonly secret: string; readonly token: string } | null;
  readonly accounts: readonly PushAccount[];
}

export const EMPTY_PUSH_STATE: PushState = { version: 1, device: null, accounts: [] };

/** A push, decrypted (`spec/push.md`, The pointer). */
export type PushPointer =
  | {
      readonly kind: "message" | "read";
      readonly channel: string;
      readonly message: string;
      readonly badge?: number;
    }
  | {
      readonly kind: "deleted";
      readonly channel: string;
      readonly message: string;
    };

const encode = (bytes: Uint8Array): string =>
  btoa(String.fromCharCode(...bytes))
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/, "");

export function base64UrlDecode(text: string): Uint8Array<ArrayBuffer> {
  const binary = atob(text.replaceAll("-", "+").replaceAll("_", "/"));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/** Makes a subscription's keys. */
export async function generatePushKeys(): Promise<PushKeys> {
  const pair = await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, [
    "deriveBits",
  ]);
  const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
  const auth = crypto.getRandomValues(new Uint8Array(16));
  return {
    privateKey: await crypto.subtle.exportKey("jwk", pair.privateKey),
    publicKey: encode(publicKey),
    auth: encode(auth),
  };
}

async function hkdf(
  salt: Uint8Array<ArrayBuffer>,
  ikm: ArrayBuffer | Uint8Array<ArrayBuffer>,
  info: Uint8Array<ArrayBuffer>,
  length: number,
): Promise<Uint8Array<ArrayBuffer>> {
  const key = await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]);
  return new Uint8Array(
    await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt, info }, key, length * 8),
  );
}

const concat = (...parts: Uint8Array[]): Uint8Array<ArrayBuffer> => {
  const out = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
};

/** Decrypts a Web Push message (RFC 8291, one `aes128gcm` record) with a subscription's keys. */
export async function decryptWebPush(
  message: Uint8Array,
  keys: PushKeys,
): Promise<Uint8Array<ArrayBuffer>> {
  const text = new TextEncoder();
  const salt = message.slice(0, 16);
  const idLength = message[20] ?? 0;
  const asPublic = message.slice(21, 21 + idLength);
  const record = message.slice(21 + idLength);
  const privateKey = await crypto.subtle.importKey(
    "jwk",
    keys.privateKey,
    { name: "ECDH", namedCurve: "P-256" },
    false,
    ["deriveBits"],
  );
  const sender = await crypto.subtle.importKey(
    "raw",
    asPublic,
    { name: "ECDH", namedCurve: "P-256" },
    false,
    [],
  );
  const secret = await crypto.subtle.deriveBits({ name: "ECDH", public: sender }, privateKey, 256);
  const uaPublic = base64UrlDecode(keys.publicKey);
  const ikm = await hkdf(
    base64UrlDecode(keys.auth),
    secret,
    concat(text.encode("WebPush: info\0"), uaPublic, asPublic),
    32,
  );
  const cek = await hkdf(salt, ikm, text.encode("Content-Encoding: aes128gcm\0"), 16);
  const nonce = await hkdf(salt, ikm, text.encode("Content-Encoding: nonce\0"), 12);
  const key = await crypto.subtle.importKey("raw", cek, "AES-GCM", false, ["decrypt"]);
  const padded = new Uint8Array(
    await crypto.subtle.decrypt({ name: "AES-GCM", iv: nonce }, key, record),
  );
  // The record ends with its delimiter (2 for the last), then any zero padding.
  let end = padded.length;
  while (end > 0 && padded[end - 1] === 0) {
    end -= 1;
  }
  if (padded[end - 1] !== 2) {
    throw new Error("a push's record does not end as the last record does");
  }
  return padded.slice(0, end - 1);
}

/** Reads a decrypted pointer; `null` for a kind or version this app does not know. */
export function parsePointer(plaintext: Uint8Array): PushPointer | null {
  const value: unknown = JSON.parse(new TextDecoder().decode(plaintext));
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const { v, kind, channel, message, badge } = value as Record<string, unknown>;
  if (v !== 1 || typeof channel !== "string" || typeof message !== "string") {
    return null;
  }
  if (kind === "deleted") {
    return { kind, channel, message };
  }
  if (kind === "message" || kind === "read") {
    return typeof badge === "number"
      ? { kind, channel, message, badge }
      : { kind, channel, message };
  }
  return null;
}

/** A relay's API (`spec/push.md`, The relay's API). */
export class RelayClient {
  readonly #url: string;
  readonly #fetch: typeof globalThis.fetch;

  constructor(url: string, fetch?: typeof globalThis.fetch) {
    this.#url = url.replace(/\/$/, "");
    this.#fetch = fetch ?? ((input, init) => globalThis.fetch(input, init));
  }

  async #call(
    method: string,
    path: string,
    body: unknown,
    secret?: string,
  ): Promise<{ status: number; json: unknown }> {
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (secret !== undefined) {
      headers.authorization = `Bearer ${secret}`;
    }
    const response = await this.#fetch(`${this.#url}${path}`, {
      method,
      headers,
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    const json: unknown = response.status === 204 ? null : await response.json().catch(() => null);
    return { status: response.status, json };
  }

  async registerDevice(device: PushDevice): Promise<{ id: string; secret: string }> {
    const { status, json } = await this.#call("POST", "/v1/devices", device);
    if (status !== 201) {
      throw new RelayError(status, json);
    }
    const { device: id, secret } = json as { device: string; secret: string };
    return { id, secret };
  }

  /** Replaces the device's platform token; `false` when the relay no longer knows it. */
  async updateToken(id: string, secret: string, token: string): Promise<boolean> {
    const { status, json } = await this.#call("PATCH", `/v1/devices/${id}`, { token }, secret);
    if (status === 404) {
      return false;
    }
    if (status !== 204) {
      throw new RelayError(status, json);
    }
    return true;
  }

  async deleteDevice(id: string, secret: string): Promise<void> {
    await this.#call("DELETE", `/v1/devices/${id}`, undefined, secret);
  }

  async subscribe(
    id: string,
    secret: string,
    applicationServerKey: string,
  ): Promise<{ subscription: string; endpoint: string }> {
    const { status, json } = await this.#call(
      "POST",
      `/v1/devices/${id}/subscriptions`,
      { applicationServerKey },
      secret,
    );
    if (status !== 201) {
      throw new RelayError(status, json);
    }
    return json as { subscription: string; endpoint: string };
  }

  async unsubscribe(id: string, secret: string, subscription: string): Promise<void> {
    await this.#call(
      "DELETE",
      `/v1/devices/${id}/subscriptions/${subscription}`,
      undefined,
      secret,
    );
  }
}

export class RelayError extends Error {
  constructor(
    readonly status: number,
    readonly problem: unknown,
  ) {
    const detail =
      typeof problem === "object" && problem !== null && "detail" in problem
        ? String(problem.detail)
        : `the relay answered ${String(status)}`;
    super(detail);
    this.name = "RelayError";
  }
}

/** One account the app is signed in to, as registering reads it. */
export interface PushSource {
  readonly origin: string;
  readonly client: AspenClient;
}

/**
 * Brings `state` in line with the phone's `device` and the accounts it is signed in to: the
 * device registered with the relay under its current token, one subscription per account whose
 * deployment wakes phones, made for that deployment's current key and registered with it, and
 * none for accounts signed out of. Answers the new state, which the caller keeps; an account
 * that could not be subscribed is left out and tried again next time.
 */
export async function syncPush(
  relay: RelayClient,
  state: PushState,
  device: PushDevice,
  sources: readonly PushSource[],
): Promise<PushState> {
  let registered = state.device;
  let accounts = state.accounts;
  if (registered !== null && registered.token !== device.token) {
    const known = await relay.updateToken(registered.id, registered.secret, device.token);
    registered = known ? { ...registered, token: device.token } : null;
  }
  if (registered === null) {
    // A new device has no subscriptions at the relay, whatever was kept.
    accounts = [];
    const { id, secret } = await relay.registerDevice(device);
    registered = { id, secret, token: device.token };
  }
  const { id, secret } = registered;
  const next: PushAccount[] = [];
  for (const { origin, client } of sources) {
    const session = client.session;
    if (session === null) {
      continue;
    }
    const kept = accounts.find((a) => a.origin === origin && a.userId === session.userId);
    const serverKey = (await client.authMethods().catch(() => null))?.push?.applicationServerKey;
    if (serverKey == null) {
      continue;
    }
    if (kept?.applicationServerKey === serverKey && kept.refreshToken === session.refreshToken) {
      next.push({ ...kept, sessionToken: session.sessionToken });
      continue;
    }
    try {
      const keys = await generatePushKeys();
      const { subscription, endpoint } = await relay.subscribe(id, secret, serverKey);
      const registeredThere = await client.registerPushSubscription({
        endpoint,
        p256dh: keys.publicKey,
        auth: keys.auth,
      });
      next.push({
        subscription,
        origin,
        userId: session.userId,
        refreshToken: session.refreshToken,
        sessionToken: session.sessionToken,
        keys,
        applicationServerKey: serverKey,
        deploymentSubscription: registeredThere.id,
      });
    } catch {
      if (kept !== undefined) {
        next.push(kept);
      }
    }
  }
  // Whatever is not kept, the signed-out and the replaced alike, is unsubscribed.
  for (const gone of accounts) {
    if (!next.some((a) => a.subscription === gone.subscription)) {
      await relay.unsubscribe(id, secret, gone.subscription).catch(() => undefined);
    }
  }
  return { version: 1, device: registered, accounts: next };
}
