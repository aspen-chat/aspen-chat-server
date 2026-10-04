import createClient, { type Client } from "openapi-fetch";
import { acceptLanguage } from "./languages";
import type { components, paths } from "./generated/openapi";
import {
  handoffPageUrl,
  pkcePair,
  PasskeyCancelledError,
  runInPage,
  type Passkey,
  type PasskeyPurpose,
  type PasskeyTransport,
} from "./passkeys";
import { ApiProblemError, isProblem, transportProblem, type Problem } from "./problem";
import { type Session, type SessionStore, sessionTokenExpiresSoon } from "./session";
import { API_PREFIX } from "./urls";

export type Paths = paths;
export type Schemas = components["schemas"];

/** The openapi-fetch client for the Aspen REST API. Every call is typed by path and method. */
export type AspenHttpClient = Client<paths>;

export interface AspenClientOptions {
  /** Server origin, e.g. `https://chat.example.org`. Paths already include `/api/v1`. */
  baseUrl: string;
  sessionStore: SessionStore;
  /** Override for tests and non-browser shells. Defaults to the global `fetch`. */
  fetch?: typeof globalThis.fetch;
  /**
   * Refresh the session token when it expires within this many milliseconds rather than waiting
   * for a 401. Defaults to one minute.
   */
  refreshLeewayMs?: number;
  /** Called after a login, refresh, or logout changes the stored session. */
  onSessionChange?: (session: Session | null) => void;
  /** Waits the given milliseconds; tests replace it. */
  sleep?: (ms: number) => Promise<void>;
}

/**
 * A read the server refuses for going too fast is tried once more after the `Retry-After` it
 * gives, if that is at most this long; a longer wait, or a refused write, is the caller's.
 */
export const RATE_LIMIT_RETRY_MAX_MS = 5_000;

export type SessionListener = (session: Session | null) => void;

export type SecondFactorMethod = Schemas["SecondFactorMethod"];
export type TypedSecondFactor = Schemas["TypedSecondFactor"];
export type ReauthenticationMethod = Schemas["ReauthenticationMethod"];
export type AuthMethods = Schemas["AuthMethods"];
export type DeploymentProfile = Schemas["DeploymentProfile"];
export type DeviceLink = Schemas["DeviceLink"];
export type DeviceLinkScan = Schemas["DeviceLinkScan"];
export type DeviceLinkProgress = Schemas["DeviceLinkProgress"];
export type RegistrationInviteRead = Schemas["Sideloaded_RegistrationInvitePreview"];

/** Where a sign-in code stands, for the device it signs in. A `signedIn` session is stored. */
export type DeviceLinkClaim =
  | { status: "waiting" }
  | { status: "scanned"; deviceName: string }
  | { status: "signedIn"; session: Session };

/** How a password sign-in ended. */
export type LoginOutcome =
  | { status: "signedIn"; session: Session }
  | { status: "secondFactorRequired"; ticket: string; methods: SecondFactorMethod[] };

/** What a passkey ceremony asks for. */
export interface PasskeyRequest {
  purpose: PasskeyPurpose;
  /** `signIn` as the second factor of a password sign-in: the ticket it left waiting. */
  ticket?: string;
  /** `register`: what to call the passkey. */
  name?: string;
}

/** How a passkey ceremony ended. A `signedIn` session is already stored. */
export type PasskeyOutcome =
  | { outcome: "signedIn"; session: Session }
  | { outcome: "passkeyAdded"; passkey: Passkey; recoveryCodes: string[] | null }
  | { outcome: "reauthenticated"; verifiedUntil: string };

/**
 * Everything a shell needs to talk to one Aspen server: a typed REST client whose requests
 * carry the session token and transparently refresh it, and the session lifecycle around it.
 *
 * Auth handling lives in a `fetch` wrapper rather than openapi-fetch middleware so a request
 * can be retried after a refresh without re-serialising its body.
 */
export class AspenClient {
  readonly api: AspenHttpClient;
  readonly baseUrl: string;

  readonly #store: SessionStore;
  readonly #fetch: typeof globalThis.fetch;
  readonly #refreshLeewayMs: number;
  readonly #onSessionChange: SessionListener | undefined;
  readonly #sleep: (ms: number) => Promise<void>;
  readonly #listeners = new Set<SessionListener>();
  /**
   * The session as last read from or written to the store. Cached so `session` returns the same
   * object until it changes, which lets React's `useSyncExternalStore` compare snapshots by
   * identity.
   */
  #session: Session | null;
  /** In-flight refresh, shared so concurrent 401s trigger one refresh instead of a stampede. */
  #refreshing: Promise<boolean> | null = null;

  constructor(options: AspenClientOptions) {
    this.baseUrl = options.baseUrl;
    this.#store = options.sessionStore;
    const fetch = options.fetch ?? ((input, init) => globalThis.fetch(input, init));
    this.#fetch = (input, init) => {
      const languages = acceptLanguage();
      if (languages === null) {
        return fetch(input, init);
      }
      // Set in place: rebuilding a request would have to copy a body that is a stream.
      if (input instanceof Request) {
        input.headers.set("accept-language", languages);
        return fetch(input, init);
      }
      const headers = new Headers(init?.headers);
      headers.set("accept-language", languages);
      return fetch(input, { ...init, headers });
    };
    this.#refreshLeewayMs = options.refreshLeewayMs ?? 60_000;
    this.#onSessionChange = options.onSessionChange;
    this.#sleep =
      options.sleep ?? ((ms) => new Promise((resolve) => globalThis.setTimeout(resolve, ms)));
    this.#session = this.#store.load();
    this.api = createClient<paths>({
      baseUrl: this.baseUrl,
      fetch: (request) => this.#authenticatedFetch(request),
      // The server declares array query parameters such as `include` with OpenAPI's
      // `style: form, explode: false`, so they travel as one comma-separated value.
      querySerializer: { array: { style: "form", explode: false } },
    });
  }

  get session(): Session | null {
    return this.#session;
  }

  /**
   * Registers for session changes and returns the unsubscribe function. Shaped for React's
   * `useSyncExternalStore(client.subscribe, () => client.session)`.
   */
  readonly subscribe = (listener: SessionListener): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  get isLoggedIn(): boolean {
    return this.session !== null;
  }

  /**
   * Signs in with a password. An account with two-factor sign-in on is not signed in yet: the
   * outcome carries the ticket to finish with `completeSecondFactor` or a passkey. Throws
   * `ApiProblemError` on failure.
   */
  async login(username: string, password: string): Promise<LoginOutcome> {
    const { data, error, response } = await this.api.POST(`${API_PREFIX}/auth/login`, {
      body: { username, password },
    });
    if (data === undefined) {
      throw new ApiProblemError(problemOf(error, response));
    }
    if (data.status === "secondFactorRequired") {
      return { status: "secondFactorRequired", ticket: data.ticket, methods: data.methods };
    }
    return { status: "signedIn", session: this.#adopt(data) };
  }

  /** Finishes a password sign-in with an authenticator code or a recovery code. */
  async completeSecondFactor(
    ticket: string,
    method: TypedSecondFactor,
    code: string,
  ): Promise<Session> {
    const response = await this.api.POST(`${API_PREFIX}/auth/login/second-factor`, {
      body: { ticket, method, code },
    });
    return this.#adopt(unwrap(response));
  }

  /** How this server lets people sign in: whether it offers passkeys, and on which domain. */
  async authMethods(): Promise<AuthMethods> {
    return unwrap(await this.api.GET(`${API_PREFIX}/auth/methods`));
  }

  /** How the deployment presents itself: its display name and icon. Needs no session. */
  async deploymentProfile(): Promise<DeploymentProfile> {
    return unwrap(await this.api.GET(`${API_PREFIX}/deployment`));
  }

  /**
   * A usable registration invite, with the community a dual invite joins sideloaded. Needs no
   * session. Throws `ApiProblemError` (`notFound`) for one that is unknown or no longer works.
   */
  async registrationInvite(code: string): Promise<RegistrationInviteRead> {
    return unwrap(
      await this.api.GET(`${API_PREFIX}/registration-invites/{code}`, {
        params: { path: { code }, query: { include: ["community"] } },
      }),
    );
  }

  /**
   * Starts a sign-in code (`/auth/device-links`). Signed in, it offers this account to a phone
   * that scans it; signed out, it asks a signed-in phone for a sign-in, naming this device
   * `deviceName`, and `verifier` is what to claim it with (`claimDeviceLink`).
   */
  async startDeviceLink(
    deviceName?: string,
  ): Promise<{ link: DeviceLink; verifier: string | null }> {
    if (this.session !== null) {
      return {
        link: unwrap(await this.api.POST(`${API_PREFIX}/auth/device-links`, { body: {} })),
        verifier: null,
      };
    }
    const { verifier, challenge } = await pkcePair();
    const link = unwrap(
      await this.api.POST(`${API_PREFIX}/auth/device-links`, {
        body: { codeChallenge: challenge, ...(deviceName === undefined ? {} : { deviceName }) },
      }),
    );
    return { link, verifier };
  }

  /**
   * Scans a sign-in code. Signed in, it grants a device asking for a sign-in (confirm with
   * `approveDeviceLink`); signed out, it asks to be signed in, naming this device `deviceName`,
   * and `verifier` is what to claim with.
   */
  async scanDeviceLink(
    id: string,
    deviceName: string,
  ): Promise<{ scan: DeviceLinkScan; verifier: string | null }> {
    if (this.session !== null) {
      const scan = unwrap(
        await this.api.POST(`${API_PREFIX}/auth/device-links/{link}/scan`, {
          params: { path: { link: id } },
          body: {},
        }),
      );
      return { scan, verifier: null };
    }
    const { verifier, challenge } = await pkcePair();
    const scan = unwrap(
      await this.api.POST(`${API_PREFIX}/auth/device-links/{link}/scan`, {
        params: { path: { link: id } },
        body: { deviceName, codeChallenge: challenge },
      }),
    );
    return { scan, verifier };
  }

  /** Where a sign-in code this signed-in device shows stands. */
  async deviceLinkProgress(id: string): Promise<DeviceLinkProgress> {
    return unwrap(
      await this.api.GET(`${API_PREFIX}/auth/device-links/{link}`, {
        params: { path: { link: id } },
      }),
    );
  }

  /** Confirms the device a sign-in code is for, signing it in to this account. */
  async approveDeviceLink(id: string): Promise<void> {
    const result = await this.api.PUT(`${API_PREFIX}/auth/device-links/{link}/approval`, {
      params: { path: { link: id } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Ends a sign-in code before it signs anyone in: declining it, or giving up on it. */
  async cancelDeviceLink(id: string): Promise<void> {
    const result = await this.api.DELETE(`${API_PREFIX}/auth/device-links/{link}`, {
      params: { path: { link: id } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Asks for the sign-in a code promises this device, with the verifier it was started or
   * scanned with; once the other device confirms, the session is stored.
   */
  async claimDeviceLink(id: string, verifier: string): Promise<DeviceLinkClaim> {
    const claimed = unwrap(
      await this.api.POST(`${API_PREFIX}/auth/device-links/{link}/claim`, {
        params: { path: { link: id } },
        body: { codeVerifier: verifier },
      }),
    );
    switch (claimed.status) {
      case "signedIn":
        return { status: "signedIn", session: this.#adopt(claimed) };
      case "scanned":
        return { status: "scanned", deviceName: claimed.deviceName };
      case "waiting":
        return { status: "waiting" };
    }
  }

  /**
   * Proves again who the user is, so the session may change security settings for a while.
   * Returns until when.
   */
  async reauthenticate(method: ReauthenticationMethod, secret: string): Promise<string> {
    const response = await this.api.POST(`${API_PREFIX}/auth/reauthenticate`, {
      body: { method, secret },
    });
    return unwrap(response).verifiedUntil;
  }

  /**
   * Runs a passkey ceremony end to end: in this page, or handed to the server's page in the
   * system browser. Throws `PasskeyCancelledError` when the user backs out, and
   * `ApiProblemError` when the server refuses.
   */
  async runPasskeyCeremony(
    request: PasskeyRequest,
    transport: PasskeyTransport,
  ): Promise<PasskeyOutcome> {
    if (transport.kind === "inPage") {
      const started = unwrap(
        await this.api.POST(`${API_PREFIX}/auth/passkey-ceremonies`, { body: request }),
      );
      const credential = await runInPage(started.purpose, started.options);
      const completed = unwrap(
        await this.api.POST(`${API_PREFIX}/auth/passkey-ceremonies/{ceremony}/credential`, {
          params: { path: { ceremony: started.id } },
          body: { credential },
        }),
      );
      return this.#passkeyOutcome(completed);
    }
    const handoff = await transport.handoff.prepare();
    try {
      const { verifier, challenge } = await pkcePair();
      const started = unwrap(
        await this.api.POST(`${API_PREFIX}/auth/passkey-ceremonies`, {
          body: { ...request, handoff: { codeChallenge: challenge, returnTo: handoff.returnTo } },
        }),
      );
      const returned = await handoff.open(handoffPageUrl(this.baseUrl, started.id));
      if (returned.ceremony !== started.id || returned.outcome === "cancelled") {
        throw new PasskeyCancelledError();
      }
      const claimed = unwrap(
        await this.api.POST(`${API_PREFIX}/auth/passkey-ceremonies/{ceremony}/claim`, {
          params: { path: { ceremony: started.id } },
          body: { codeVerifier: verifier },
        }),
      );
      return this.#passkeyOutcome(claimed);
    } finally {
      handoff.dispose();
    }
  }

  #passkeyOutcome(result: Schemas["PasskeyCeremonyOutcome"]): PasskeyOutcome {
    switch (result.outcome) {
      case "signedIn":
        return { outcome: "signedIn", session: this.#adopt(result) };
      case "passkeyAdded":
        return {
          outcome: "passkeyAdded",
          passkey: result.passkey,
          recoveryCodes: result.recoveryCodes ?? null,
        };
      case "reauthenticated":
        return { outcome: "reauthenticated", verifiedUntil: result.verifiedUntil };
      case "handedOff":
        // Only a handed-off completion answers this, and the page, not the app, receives it.
        throw new Error("unexpected handedOff outcome");
    }
  }

  /**
   * Records that the account now has a second factor, lifting a server's requirement that it
   * add one. The app calls it once the user has seen the recovery codes that came with the
   * first factor, since lifting the requirement takes the enrollment screen, and anything it
   * shows, away.
   */
  markEnrolled(): void {
    const session = this.session;
    if (session?.twoFactorEnrollmentRequired === true) {
      this.#setSession({ ...session, twoFactorEnrollmentRequired: false });
    }
  }

  /** Stores the credentials of a completed sign-in. */
  #adopt(response: Schemas["LoginResponse"]): Session {
    const session: Session = {
      userId: response.userId,
      refreshToken: response.refreshToken,
      sessionToken: response.sessionToken,
      sessionTokenExpires: response.sessionTokenExpires,
      twoFactorEnrollmentRequired: response.twoFactorEnrollmentRequired,
    };
    this.#setSession(session);
    return session;
  }

  /**
   * Signs an assertion that the signed-in user is who they are, for them to sign in at
   * `audience`, another deployment, with `signInWithAssertion` there. Throws `ApiProblemError`
   * (`federationRefused`) when this deployment does not let its users go there.
   */
  async issueAssertion(audience: string): Promise<Schemas["Issued"]> {
    return unwrap(await this.api.POST(`${API_PREFIX}/auth/assertions`, { body: { audience } }));
  }

  /**
   * Signs in a user of another deployment with an assertion their home signed for this one,
   * and stores the session. `inviteCode` is a registration invite, which a first arrival needs
   * where this deployment asks one of accounts from elsewhere.
   */
  async signInWithAssertion(assertion: string, inviteCode?: string): Promise<Session> {
    const response = await this.api.POST(`${API_PREFIX}/auth/federated-sign-in`, {
      body: { assertion, ...(inviteCode === undefined ? {} : { inviteCode }) },
    });
    return this.#adopt(unwrap(response));
  }

  /** The other deployments the signed-in user has signed in to from here, most recent first. */
  async foreignDeployments(): Promise<Schemas["ForeignDeployment"][]> {
    return unwrap(await this.api.GET(`${API_PREFIX}/users/@me/foreign-deployments`));
  }

  /** Stops the signed-in user's devices signing in at `domain`. */
  async forgetForeignDeployment(domain: string): Promise<void> {
    const result = await this.api.DELETE(`${API_PREFIX}/users/@me/foreign-deployments/{domain}`, {
      params: { path: { domain } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Registers this sign-in's phone to be woken (`spec/push.md`): the endpoint its relay gave
   * it, and its keys, base64url.
   */
  async registerPushSubscription(
    body: Schemas["PushSubscriptionRequest"],
  ): Promise<Schemas["PushSubscription"]> {
    const result = await this.api.POST(`${API_PREFIX}/users/@me/push-subscriptions`, { body });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    return result.data;
  }

  /**
   * Forgets the session locally without telling the server, as when another deployment's
   * session is abandoned and its server may be unreachable.
   */
  forgetSession(): void {
    this.#setSession(null);
  }

  /**
   * Creates an account. Registration does not sign the user in; callers typically follow it
   * with `login`. Throws `ApiProblemError` (`usernameTaken`, `validation`,
   * `passwordRequirementsNotMet`) on failure.
   */
  /** Creates an account; `inviteCode` is the registration invite, which some servers require. */
  async register(name: string, password: string, inviteCode?: string): Promise<Schemas["User"]> {
    const result = await this.api.POST(`${API_PREFIX}/users`, {
      body: { name, password, ...(inviteCode === undefined ? {} : { inviteCode }) },
    });
    return unwrap(result);
  }

  /**
   * Revokes the refresh token server-side and forgets the session locally. The local state is
   * cleared even if the server cannot be reached, so the user is never stuck logged in.
   */
  async logout(): Promise<void> {
    const session = this.session;
    if (session === null) {
      return;
    }
    try {
      await this.api.POST(`${API_PREFIX}/auth/logout`, {
        body: { refreshToken: session.refreshToken },
      });
    } finally {
      this.#setSession(null);
    }
  }

  /**
   * Exchanges the refresh token for a new session token. Returns `false` (and clears the
   * session) when the refresh token itself is rejected, which means the user must log in again.
   */
  refreshSession(): Promise<boolean> {
    this.#refreshing ??= this.#refreshSessionUncached().finally(() => {
      this.#refreshing = null;
    });
    return this.#refreshing;
  }

  /**
   * The session token to present out of band, for example in the event stream's `identify`
   * frame. Refreshes first when the token is about to expire, or unconditionally when
   * `forceRefresh` is set because the previous token was just rejected. `null` means there is
   * no usable session and the user must log in again.
   */
  async freshSessionToken(options: { forceRefresh?: boolean } = {}): Promise<string | null> {
    let session = this.session;
    if (session === null) {
      return null;
    }
    if (options.forceRefresh === true || sessionTokenExpiresSoon(session, this.#refreshLeewayMs)) {
      await this.refreshSession();
      session = this.session;
    }
    return session?.sessionToken ?? null;
  }

  async #refreshSessionUncached(): Promise<boolean> {
    const session = this.session;
    if (session === null) {
      return false;
    }
    // Bypass the authenticated wrapper: this request must not itself trigger a refresh.
    const response = await this.#fetch(`${this.baseUrl}${API_PREFIX}/auth/token-refresh`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ refreshToken: session.refreshToken }),
    });
    if (response.status === 401) {
      this.#setSession(null);
      return false;
    }
    if (!response.ok) {
      // Transient server trouble; keep the session and let the caller surface the error.
      return false;
    }
    const body = (await response.json()) as Schemas["TokenRefreshResponse"];
    this.#setSession({
      ...session,
      sessionToken: body.sessionToken,
      sessionTokenExpires: body.sessionTokenExpires,
    });
    return true;
  }

  async #authenticatedFetch(request: Request): Promise<Response> {
    const retry = request.method === "GET" || request.method === "HEAD" ? request.clone() : null;
    const response = await this.#sessionFetch(request);
    if (response.status !== 429 || retry === null) {
      return response;
    }
    const waitMs = Number(response.headers.get("retry-after")) * 1000;
    if (!Number.isFinite(waitMs) || waitMs <= 0 || waitMs > RATE_LIMIT_RETRY_MAX_MS) {
      return response;
    }
    await this.#sleep(waitMs);
    return this.#sessionFetch(retry);
  }

  async #sessionFetch(request: Request): Promise<Response> {
    let session = this.session;
    if (session !== null && sessionTokenExpiresSoon(session, this.#refreshLeewayMs)) {
      await this.refreshSession();
      session = this.session;
    }
    if (session === null) {
      return this.#fetch(request);
    }
    // Keep an unread copy so the request can be replayed after a refresh.
    const retry = request.clone();
    const response = await this.#fetch(withBearer(request, session.sessionToken));
    if (response.status === 403) {
      await this.#noticeEnrollmentRequired(response);
    }
    if (response.status !== 401) {
      return response;
    }
    const refreshed = await this.refreshSession();
    const fresh = this.session;
    if (!refreshed || fresh === null) {
      return response;
    }
    return this.#fetch(withBearer(retry, fresh.sessionToken));
  }

  /**
   * A server that has started requiring a second factor answers every request of an account
   * without one this way. Flagging the session lets the app show the enrollment screen instead
   * of failing everywhere.
   */
  async #noticeEnrollmentRequired(response: Response): Promise<void> {
    let body: unknown;
    try {
      body = await response.clone().json();
    } catch {
      return;
    }
    const session = this.session;
    if (
      isProblem(body) &&
      body.code === "twoFactorEnrollmentRequired" &&
      session !== null &&
      session.twoFactorEnrollmentRequired !== true
    ) {
      this.#setSession({ ...session, twoFactorEnrollmentRequired: true });
    }
  }

  #setSession(session: Session | null): void {
    if (session === null) {
      this.#store.clear();
    } else {
      this.#store.save(session);
    }
    this.#session = session;
    this.#onSessionChange?.(session);
    for (const listener of this.#listeners) {
      listener(session);
    }
  }
}

function withBearer(request: Request, token: string): Request {
  const headers = new Headers(request.headers);
  headers.set("authorization", `Bearer ${token}`);
  return new Request(request, { headers });
}

/**
 * Extracts a Problem from an openapi-fetch failure. The server always sends one, but a proxy or
 * a dropped connection may not; those become a synthetic `internal` Problem.
 */
export function problemOf(error: unknown, response: Response | undefined): Problem {
  if (isProblem(error)) {
    return error;
  }
  const status = response?.status ?? 0;
  const statusText = response?.statusText ?? "";
  return transportProblem(
    statusText.length > 0 ? `${String(status)} ${statusText}` : "no response",
    status,
  );
}

/** Unwraps an openapi-fetch result, throwing `ApiProblemError` instead of returning `error`. */
export function unwrap<T>(result: { data?: T; error?: unknown; response: Response }): T {
  if (result.data !== undefined) {
    return result.data;
  }
  throw new ApiProblemError(problemOf(result.error, result.response));
}
