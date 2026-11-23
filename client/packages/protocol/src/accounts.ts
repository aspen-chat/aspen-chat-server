import { SESSION_KEY } from "./session";

/** The most accounts one install may hold: each keeps a sync per deployment it uses. */
export const MAX_ACCOUNTS = 5;

/** The account an install starts with, whose session keeps the key it has always had. */
export const PRIMARY_ACCOUNT = "primary";

/** One sign-in at a home deployment. `id` is this install's own, never a server's. */
export interface Account {
  readonly id: string;
  readonly serverUrl: string;
}

export interface AccountsState {
  /** Whether Settings' multiple accounts are on. */
  readonly enabled: boolean;
  /** The account last in front, which a window with none of its own opens on. */
  readonly front: string;
  readonly accounts: readonly Account[];
}

const STORAGE_KEY = "aspen.accounts";

/** Where `account`'s session at its home is kept. */
export function accountSessionKey(account: Account): string {
  return account.id === PRIMARY_ACCOUNT ? SESSION_KEY : `${SESSION_KEY}.account.${account.id}`;
}

/**
 * Where `account`'s session at another deployment is kept. Keys of added accounts are made of
 * this install's own id: a home chooses the user ids it reports, and must not be able to name
 * another account's sessions with one.
 */
export function foreignSessionKey(account: Account, homeUser: string, domain: string): string {
  return account.id === PRIMARY_ACCOUNT
    ? `${SESSION_KEY}.${homeUser}@${domain}`
    : `${accountSessionKey(account)}@${domain}`;
}

function parseAccount(raw: unknown): Account | null {
  if (raw === null || typeof raw !== "object") {
    return null;
  }
  const { id, serverUrl } = raw as Record<string, unknown>;
  return typeof id === "string" && id !== "" && typeof serverUrl === "string"
    ? { id, serverUrl }
    : null;
}

/**
 * The accounts this install holds, kept in `storage`: which there are and where, never a token.
 * An install that has not turned multiple accounts on holds one account, on the server it chose.
 */
export class AccountRegistry {
  readonly #storage: Storage;
  readonly #newId: () => string;

  constructor(storage: Storage, newId: () => string = () => crypto.randomUUID()) {
    this.#storage = storage;
    this.#newId = newId;
  }

  #kept(): { enabled: boolean; front: string | null; accounts: Account[] } {
    const none = { enabled: false, front: null, accounts: [] };
    let raw: string | null;
    try {
      raw = this.#storage.getItem(STORAGE_KEY);
    } catch {
      return none;
    }
    if (raw === null) {
      return none;
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      return none;
    }
    if (parsed === null || typeof parsed !== "object") {
      return none;
    }
    const { version, enabled, front, accounts } = parsed as Record<string, unknown>;
    if (version !== 1 || !Array.isArray(accounts)) {
      return none;
    }
    const seen = new Set<string>();
    const valid = accounts.flatMap((entry: unknown) => {
      const account = parseAccount(entry);
      if (account === null || seen.has(account.id)) {
        return [];
      }
      seen.add(account.id);
      return [account];
    });
    return {
      enabled: enabled === true,
      front: typeof front === "string" ? front : null,
      accounts: valid.slice(0, MAX_ACCOUNTS),
    };
  }

  #keep(state: AccountsState): void {
    try {
      this.#storage.setItem(STORAGE_KEY, JSON.stringify({ version: 1, ...state }));
    } catch (error) {
      console.error("accounts: the list could not be kept", error);
    }
  }

  /**
   * The state as it stands, for an install whose chosen server is `chosenServer`. With multiple
   * accounts off there is one account, on that server.
   */
  state(chosenServer: string): AccountsState {
    const kept = this.#kept();
    const first = kept.accounts[0] ?? { id: PRIMARY_ACCOUNT, serverUrl: chosenServer };
    const accounts = kept.enabled
      ? kept.accounts.length === 0
        ? [first]
        : kept.accounts
      : [{ id: first.id, serverUrl: chosenServer }];
    const front = accounts.find((account) => account.id === kept.front) ?? accounts[0] ?? first;
    return { enabled: kept.enabled, front: front.id, accounts };
  }

  enable(chosenServer: string): void {
    this.#keep({ ...this.state(chosenServer), enabled: true });
  }

  /** Turns multiple accounts off, keeping `keep` alone. */
  disable(chosenServer: string, keep: string): void {
    const state = this.state(chosenServer);
    const kept = state.accounts.find((account) => account.id === keep) ?? state.accounts[0];
    this.#keep({
      enabled: false,
      front: kept?.id ?? PRIMARY_ACCOUNT,
      accounts: kept === undefined ? [] : [kept],
    });
  }

  /** An account not yet held, for signing in to. */
  draft(serverUrl: string): Account {
    return { id: this.#newId(), serverUrl };
  }

  /** Holds `account`, unless the install already holds as many as it may. */
  add(chosenServer: string, account: Account): boolean {
    const state = this.state(chosenServer);
    if (state.accounts.length >= MAX_ACCOUNTS) {
      return false;
    }
    this.#keep({ ...state, accounts: [...state.accounts, account] });
    return true;
  }

  /** Stops holding `id`. The last account stays, for signing in to again. */
  remove(chosenServer: string, id: string): void {
    const state = this.state(chosenServer);
    const accounts = state.accounts.filter((account) => account.id !== id);
    if (accounts.length === 0) {
      return;
    }
    this.#keep({ ...state, accounts });
  }

  setFront(chosenServer: string, id: string): void {
    this.#keep({ ...this.state(chosenServer), front: id });
  }
}
