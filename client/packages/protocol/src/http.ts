import createClient, { type Client } from "openapi-fetch";
import type { components, paths } from "./generated/openapi";
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
}

export type SessionListener = (session: Session | null) => void;

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
    this.#fetch = options.fetch ?? ((input, init) => globalThis.fetch(input, init));
    this.#refreshLeewayMs = options.refreshLeewayMs ?? 60_000;
    this.#onSessionChange = options.onSessionChange;
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

  /** Logs in and stores the resulting session. Throws `ApiProblemError` on failure. */
  async login(username: string, password: string): Promise<Session> {
    const { data, error, response } = await this.api.POST(`${API_PREFIX}/auth/login`, {
      body: { username, password },
    });
    if (data === undefined) {
      throw new ApiProblemError(problemOf(error, response));
    }
    const session: Session = {
      userId: data.userId,
      refreshToken: data.refreshToken,
      sessionToken: data.sessionToken,
      sessionTokenExpires: data.sessionTokenExpires,
    };
    this.#setSession(session);
    return session;
  }

  /**
   * Creates an account. Registration does not sign the user in; callers typically follow it
   * with `login`. Throws `ApiProblemError` (`usernameTaken`, `validation`,
   * `passwordRequirementsNotMet`) on failure.
   */
  async register(name: string, password: string): Promise<Schemas["User"]> {
    const result = await this.api.POST(`${API_PREFIX}/users`, { body: { name, password } });
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
