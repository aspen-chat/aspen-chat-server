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
  /**
   * The server requires a verified email address and this account's is not verified. Until it
   * is, the session can do nothing but verify, change, or resend it, or sign out.
   */
  emailVerificationRequired?: boolean;
}

/**
 * Where a session is persisted: web storage in a browser tab, which has no key of the system's to
 * encrypt with, and a `SessionVault` in the desktop and mobile apps.
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

/** The key under which a session is kept when no other is given. */
export const SESSION_KEY = "aspen.session";

/**
 * What reading a vault's contents answered. `unavailable`: this device has no system store to
 * encrypt with. `unreadable`: contents are kept that the system's key does not open, or the
 * system refused to open it (a keyring left locked). `contents` is `null` when none are kept.
 */
export type VaultRead =
  | { readonly status: "ok"; readonly contents: string | null }
  | { readonly status: "unavailable" }
  | { readonly status: "unreadable" };

/** A shell's encrypted place for a vault's contents, one text replaced whole. */
export interface VaultBackend {
  read(): Promise<VaultRead>;
  /** Whether `contents` were kept. */
  write(contents: string): Promise<boolean>;
}

export type VaultOpening =
  | { readonly status: "open"; readonly vault: SessionVault }
  | { readonly status: "unavailable" }
  | { readonly status: "unreadable" };

function isLegacySessionKey(key: string): boolean {
  return key === SESSION_KEY || key.startsWith(`${SESSION_KEY}.`);
}

/** Removes every session kept in `storage` unencrypted. */
export function dropLegacySessions(storage: Storage): void {
  const keys: string[] = [];
  for (let index = 0; index < storage.length; index += 1) {
    const key = storage.key(index);
    if (key !== null && isLegacySessionKey(key)) {
      keys.push(key);
    }
  }
  for (const key of keys) {
    storage.removeItem(key);
  }
}

/**
 * Every session of an app, held in memory and kept encrypted by the system. `AspenClient` reads
 * its session synchronously as it is made, so the vault is opened once before any client is, and
 * each change is written behind, in order, the latest replacing any not yet begun. A write that
 * fails leaves the session in memory for this run and is logged; the next change tries again.
 */
export class SessionVault {
  readonly #sessions = new Map<string, Session>();
  readonly #backend: VaultBackend | null;
  #dirty = false;
  #lastWriteFailed = false;
  #writing: Promise<void> | null = null;

  private constructor(backend: VaultBackend | null) {
    this.#backend = backend;
  }

  /**
   * Opens what `backend` keeps. Sessions found unencrypted in `legacy` move into the vault and
   * leave `legacy` only once the vault has kept them; one the vault already holds is newer.
   */
  static async open(backend: VaultBackend, legacy?: Storage): Promise<VaultOpening> {
    const read = await backend.read();
    if (read.status !== "ok") {
      return read;
    }
    const vault = new SessionVault(backend);
    vault.#restore(read.contents);
    if (legacy !== undefined && vault.#adopt(legacy)) {
      vault.#persist();
      await vault.settled();
      if (!vault.#lastWriteFailed) {
        dropLegacySessions(legacy);
      }
    } else if (legacy !== undefined) {
      dropLegacySessions(legacy);
    }
    return { status: "open", vault };
  }

  /** A vault that replaces whatever `backend` keeps with nothing, for starting over. */
  static async emptied(backend: VaultBackend): Promise<SessionVault> {
    const vault = new SessionVault(backend);
    vault.#persist();
    await vault.settled();
    return vault;
  }

  /** A vault that keeps nothing past this run. */
  static inMemory(): SessionVault {
    return new SessionVault(null);
  }

  #restore(contents: string | null): void {
    if (contents === null) {
      return;
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(contents);
    } catch {
      return;
    }
    if (parsed === null || typeof parsed !== "object") {
      return;
    }
    for (const [key, value] of Object.entries(parsed)) {
      if (isSession(value)) {
        this.#sessions.set(key, value);
      }
    }
  }

  /** Whether `legacy` gave the vault a session it lacked. */
  #adopt(legacy: Storage): boolean {
    let adopted = false;
    for (let index = 0; index < legacy.length; index += 1) {
      const key = legacy.key(index);
      if (key === null || !isLegacySessionKey(key) || this.#sessions.has(key)) {
        continue;
      }
      const session = new WebStorageSessionStore(legacy, key).load();
      if (session !== null) {
        this.#sessions.set(key, session);
        adopted = true;
      }
    }
    return adopted;
  }

  /** The store of one session, under the key web storage would keep it by. */
  store(key: string = SESSION_KEY): SessionStore {
    return {
      load: () => this.#sessions.get(key) ?? null,
      save: (session) => {
        this.#sessions.set(key, session);
        this.#persist();
      },
      clear: () => {
        if (this.#sessions.delete(key)) {
          this.#persist();
        }
      },
    };
  }

  /** Resolves once every change so far has been written or has failed to be. */
  settled(): Promise<void> {
    return this.#writing ?? Promise.resolve();
  }

  #persist(): void {
    if (this.#backend === null) {
      return;
    }
    this.#dirty = true;
    this.#writing ??= this.#drain(this.#backend);
  }

  async #drain(backend: VaultBackend): Promise<void> {
    while (this.#dirty) {
      this.#dirty = false;
      const contents = JSON.stringify(Object.fromEntries(this.#sessions));
      const kept = await backend.write(contents).catch((error: unknown) => {
        console.error("sessions: the system's store refused them", error);
        return false;
      });
      this.#lastWriteFailed = !kept;
      if (!kept) {
        console.error("sessions: not kept; they last until the app closes");
      }
    }
    this.#writing = null;
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

/**
 * Whether the session token expires within `withinMs` of `now`, a reading of the clock of the
 * server that issued it (`ServerClock`), which stamped the expiry.
 */
export function sessionTokenExpiresSoon(session: Session, withinMs: number, now: number): boolean {
  const expires = Date.parse(session.sessionTokenExpires);
  return Number.isNaN(expires) || expires - now <= withinMs;
}
