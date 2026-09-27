/**
 * Credentials issued by `POST /api/v1/auth/login`, kept between requests and across restarts.
 *
 * `refreshToken` lives for about a year and must be stored as securely as the platform allows;
 * `sessionToken` lives for hours and is what every request carries as `Authorization: Bearer`.
 */
export interface Session {
  userId: string;
  refreshToken: string;
  sessionToken: string;
  /** RFC 3339 timestamp. */
  sessionTokenExpires: string;
  /**
   * The server requires a second factor this account has not added. Until it adds one, the
   * session can do nothing but add one or sign out.
   */
  twoFactorEnrollmentRequired?: boolean;
}

/**
 * Where a session is persisted. Each shell supplies the right implementation: web storage in a
 * browser tab, Electron `safeStorage` on the desktop, the keychain on mobile.
 */
export interface SessionStore {
  load(): Session | null;
  save(session: Session): void;
  clear(): void;
}

export class MemorySessionStore implements SessionStore {
  #session: Session | null = null;

  load(): Session | null {
    return this.#session;
  }

  save(session: Session): void {
    this.#session = session;
  }

  clear(): void {
    this.#session = null;
  }
}

/** Persists the session as JSON under one key of a `Storage` (localStorage or sessionStorage). */
export class WebStorageSessionStore implements SessionStore {
  readonly #storage: Storage;
  readonly #key: string;

  constructor(storage: Storage, key = "aspen.session") {
    this.#storage = storage;
    this.#key = key;
  }

  load(): Session | null {
    let raw: string | null;
    try {
      raw = this.#storage.getItem(this.#key);
    } catch {
      return null;
    }
    if (raw === null) {
      return null;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      return isSession(parsed) ? parsed : null;
    } catch {
      return null;
    }
  }

  save(session: Session): void {
    this.#storage.setItem(this.#key, JSON.stringify(session));
  }

  clear(): void {
    this.#storage.removeItem(this.#key);
  }
}

export function isSession(value: unknown): value is Session {
  if (value === null || typeof value !== "object") {
    return false;
  }
  const v = value as Record<string, unknown>;
  return (
    typeof v.userId === "string" &&
    typeof v.refreshToken === "string" &&
    typeof v.sessionToken === "string" &&
    typeof v.sessionTokenExpires === "string"
  );
}

/** Whether the session token expires within `withinMs` of `now`. */
export function sessionTokenExpiresSoon(
  session: Session,
  withinMs: number,
  now: number = Date.now(),
): boolean {
  const expires = Date.parse(session.sessionTokenExpires);
  return Number.isNaN(expires) || expires - now <= withinMs;
}
