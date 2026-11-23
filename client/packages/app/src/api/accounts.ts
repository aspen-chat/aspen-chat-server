import {
  AccountRegistry,
  AspenClient,
  accountSessionKey,
  type Account,
  type AccountsState,
} from "@aspen/protocol";
import { createContext, useContext, useSyncExternalStore } from "react";
import { clockStore, deviceStore, forgetDeviceCredential, solveInWorkers } from "./devices";
import type { Source } from "./everywhere";
import { sessionStore } from "./sessions";

export interface AccountEntry {
  readonly account: Account;
  readonly client: AspenClient;
}

/** What a signed-in account offers those outside it, while its sync runs. */
export interface AccountRuntime {
  readonly sources: readonly Source[];
  signOut(): Promise<void>;
}

export interface AccountsView {
  readonly enabled: boolean;
  readonly front: string;
  readonly entries: readonly AccountEntry[];
}

/** The account this window shows, kept per window so that two tabs may show two. */
const FRONT_KEY = "aspen.accounts.front";

function windowFront(): string | null {
  try {
    return window.sessionStorage.getItem(FRONT_KEY);
  } catch {
    return null;
  }
}

function keepWindowFront(id: string): void {
  try {
    window.sessionStorage.setItem(FRONT_KEY, id);
  } catch {
    // The install's own record still says which account was last in front.
  }
}

function makeClient(account: Account): AspenClient {
  return new AspenClient({
    baseUrl: account.serverUrl,
    sessionStore: sessionStore(accountSessionKey(account)),
    deviceStore: deviceStore(account.serverUrl, account.id),
    clockStore: clockStore(account.serverUrl),
    solvePuzzle: solveInWorkers,
  });
}

/**
 * The accounts this install holds, each with one client for as long as the page lives, and which
 * is in front. The rest stay signed in and connected behind it.
 */
export class AccountsHub {
  readonly chosenServer: string;
  readonly #registry: AccountRegistry;
  readonly #clients = new Map<string, AspenClient>();
  readonly #runtimes = new Map<string, AccountRuntime>();
  readonly #routes = new Map<string, string>();
  readonly #listeners = new Set<() => void>();
  #front: string | null = windowFront();
  #view: AccountsView;
  #sources: readonly Source[] = [];

  constructor(chosenServer: string, registry = new AccountRegistry(window.localStorage)) {
    this.chosenServer = chosenServer;
    this.#registry = registry;
    this.#view = this.#read();
  }

  #clientOf(account: Account): AspenClient {
    let client = this.#clients.get(account.id);
    if (client === undefined) {
      client = makeClient(account);
      this.#clients.set(account.id, client);
    }
    return client;
  }

  #state(): AccountsState {
    return this.#registry.state(this.chosenServer);
  }

  #read(): AccountsView {
    const state = this.#state();
    const front = state.accounts.find((account) => account.id === this.#front)?.id ?? state.front;
    this.#front = front;
    return {
      enabled: state.enabled,
      front,
      entries: state.accounts.map((account) => ({ account, client: this.#clientOf(account) })),
    };
  }

  #changed(): void {
    this.#view = this.#read();
    this.#tell();
  }

  #tell(): void {
    for (const listener of this.#listeners) {
      listener();
    }
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  readonly view = (): AccountsView => this.#view;

  /** Every deployment of every signed-in account. The same array until one changes. */
  readonly allSources = (): readonly Source[] => this.#sources;

  runtime(id: string): AccountRuntime | undefined {
    return this.#runtimes.get(id);
  }

  isFront(id: string): boolean {
    return this.#view.front === id;
  }

  /** Whether more than one account is held, so what is shown must say whose it is. */
  get several(): boolean {
    return this.#view.entries.length > 1;
  }

  publish(id: string, runtime: AccountRuntime | null): void {
    if (runtime === null) {
      this.#runtimes.delete(id);
    } else {
      this.#runtimes.set(id, runtime);
    }
    this.#sources = this.#view.entries.flatMap(
      ({ account }) => this.#runtimes.get(account.id)?.sources ?? [],
    );
    this.#tell();
  }

  /** Remembers where the account in front is, for coming back to it. */
  noteRoute(href: string): void {
    this.#routes.set(this.#view.front, href);
  }

  routeOf(id: string): string | undefined {
    return this.#routes.get(id);
  }

  bringToFront(id: string): void {
    if (this.#view.front === id || !this.#view.entries.some((e) => e.account.id === id)) {
      return;
    }
    this.#front = id;
    keepWindowFront(id);
    this.#registry.setFront(this.chosenServer, id);
    this.#changed();
  }

  enable(): void {
    this.#registry.enable(this.chosenServer);
    this.#changed();
  }

  /** Signs out of every account but the one in front, and goes back to holding one. */
  async disable(): Promise<void> {
    const keep = this.#view.front;
    const others = this.#view.entries.filter(({ account }) => account.id !== keep);
    await Promise.all(others.map(({ account }) => this.#end(account.id)));
    this.#registry.disable(this.chosenServer, keep);
    for (const { account } of others) {
      this.#forget(account);
    }
    this.#changed();
  }

  /** An account to sign in to, not held until `commit`. */
  draft(serverUrl: string): AccountEntry {
    const account = this.#registry.draft(serverUrl);
    return { account, client: this.#clientOf(account) };
  }

  /** Holds a signed-in draft and brings it to the front; `false` when no more may be held. */
  commit({ account }: AccountEntry): boolean {
    if (!this.#registry.add(this.chosenServer, account)) {
      return false;
    }
    this.#changed();
    this.bringToFront(account.id);
    return true;
  }

  /** Drops a draft that was not signed in to, or was found to be an account already held. */
  discard({ account, client }: AccountEntry): void {
    if (client.session !== null) {
      void client.logout().catch(() => {
        client.forgetSession();
      });
    }
    this.#forget(account);
  }

  #forget(account: Account): void {
    this.#clients.delete(account.id);
    this.#routes.delete(account.id);
    forgetDeviceCredential(account.serverUrl, account.id);
  }

  async #end(id: string): Promise<void> {
    const runtime = this.#runtimes.get(id);
    if (runtime !== undefined) {
      await runtime.signOut();
      return;
    }
    const client = this.#clients.get(id);
    await client?.logout().catch(() => {
      client.forgetSession();
    });
  }

  /** Signs out of `id`. With others held it goes from the list; alone it stays to sign in to. */
  async signOut(id: string): Promise<void> {
    await this.#end(id);
    const entry = this.#view.entries.find(({ account }) => account.id === id);
    if (entry === undefined || this.#view.entries.length === 1) {
      return;
    }
    if (this.#front === id) {
      this.#front = null;
    }
    this.#registry.remove(this.chosenServer, id);
    this.#forget(entry.account);
    this.#changed();
  }
}

export const AccountsContext = createContext<AccountsHub | null>(null);

/** This install's accounts. Only valid inside `<AspenProvider>`. */
export function useAccountsHub(): AccountsHub {
  const hub = useContext(AccountsContext);
  if (hub === null) {
    throw new Error("useAccountsHub must be used inside <AspenProvider>");
  }
  return hub;
}

export function useAccounts(): AccountsView {
  const hub = useAccountsHub();
  return useSyncExternalStore(hub.subscribe, hub.view);
}

/** Every deployment of every signed-in account, for what the device does once. */
export function useAllSources(): readonly Source[] {
  const hub = useAccountsHub();
  return useSyncExternalStore(hub.subscribe, hub.allSources);
}

/** The account whose scope this is. */
export const AccountContext = createContext<Account | null>(null);

export function useAccount(): Account {
  const account = useContext(AccountContext);
  if (account === null) {
    throw new Error("useAccount must be used inside an account's scope");
  }
  return account;
}
