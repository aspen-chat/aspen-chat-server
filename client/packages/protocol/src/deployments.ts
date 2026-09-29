import type { AspenClient } from "./http";
import { ApiProblemError } from "./problem";
import { CLIENT_PROTOCOL, commonVersion } from "./protocol";
import type { AspenSync } from "./sync";

/**
 * Where signing in at another deployment stands. `incompatible`: it speaks no version of the
 * Aspen protocol this client does (`spec/federation.md`), so it is not signed in to.
 */
export type DeploymentStatus = "connecting" | "ready" | "failed" | "incompatible";

/** Another deployment the user signs in to from their home, with its own client and sync. */
export interface ForeignDeployment {
  /** Its domain, with `:port` when not 443: how the home names it, and where its API is. */
  readonly domain: string;
  readonly status: DeploymentStatus;
  /** Why signing in there failed, as its server or the home put it; `null` otherwise. */
  readonly problem: string | null;
  readonly client: AspenClient;
  /** `null` until signed in there. */
  readonly sync: AspenSync | null;
}

export interface DeploymentsOptions {
  /** The client of the user's home deployment, signed in. */
  home: AspenClient;
  /** Makes the client for another deployment, with a session store of that deployment's own. */
  client: (domain: string) => AspenClient;
  /** Makes the sync for another deployment's client. */
  sync: (client: AspenClient) => AspenSync;
  now?: () => number;
}

export type DeploymentsListener = () => void;

/** How soon after one a lost session abroad may be replaced again, so a refusal cannot loop. */
export const REACQUIRE_INTERVAL_MS = 30_000;

/** Another deployment speaks no version of the Aspen protocol this client does. */
export class IncompatibleDeploymentError extends Error {
  constructor(readonly domain: string) {
    super(`${domain} speaks no version of the Aspen protocol this client does`);
    this.name = "IncompatibleDeploymentError";
  }
}

/** The API origin of the deployment named `domain`. */
export function deploymentUrl(domain: string): string {
  return `https://${domain}`;
}

interface Entry {
  domain: string;
  status: DeploymentStatus;
  problem: string | null;
  client: AspenClient;
  sync: AspenSync | null;
  /** Set while the deployment is being left, so a session ending then is not replaced. */
  leaving: boolean;
  lastAcquiredAt: number;
  unsubscribe: () => void;
}

/**
 * The other deployments a user signs in to from their home: the home keeps the list, so each
 * of their devices signs in to the same ones. A session abroad comes from an assertion the home
 * signs for that deployment; when one ends (it expired, or the deployment revoked it), another
 * is fetched the same way.
 */
export class Deployments {
  readonly #options: DeploymentsOptions;
  readonly #now: () => number;
  readonly #entries = new Map<string, Entry>();
  readonly #listeners = new Set<DeploymentsListener>();
  #snapshot: readonly ForeignDeployment[] = [];
  /** Increments on every start and stop, so a stale async step can notice and bail. */
  #generation = 0;

  constructor(options: DeploymentsOptions) {
    this.#options = options;
    this.#now = options.now ?? (() => Date.now());
  }

  /** Every other deployment, in the order they were opened. The same array until one changes. */
  get list(): readonly ForeignDeployment[] {
    return this.#snapshot;
  }

  get(domain: string): ForeignDeployment | undefined {
    return this.#snapshot.find((entry) => entry.domain === domain);
  }

  /** Registers for changes and returns the unsubscribe function. */
  readonly subscribe = (listener: DeploymentsListener): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  /** Signs in to every deployment the home lists. A home it cannot read lists none. */
  async start(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    let listed: { domain: string }[];
    try {
      listed = await this.#options.home.foreignDeployments();
    } catch {
      return;
    }
    if (generation !== this.#generation) {
      return;
    }
    await Promise.all(
      listed.map(async ({ domain }) => {
        await this.#open(domain, undefined, generation).catch(() => undefined);
      }),
    );
  }

  /**
   * Signs in at `domain` for the first time from this device, or returns it when already
   * signed in there. Throws `ApiProblemError` when the home or `domain` refuses.
   */
  async join(domain: string, inviteCode?: string): Promise<ForeignDeployment> {
    const known = this.#entries.get(domain);
    if (known?.status === "ready") {
      return this.#view(known);
    }
    return this.#open(domain, inviteCode, this.#generation);
  }

  /**
   * Stops using `domain`: signs out there and asks the home to forget it, so the user's other
   * devices stop too.
   */
  async leave(domain: string): Promise<void> {
    const entry = this.#entries.get(domain);
    if (entry !== undefined) {
      entry.leaving = true;
      this.#close(entry);
      await entry.client.logout().catch(() => {
        entry.client.forgetSession();
      });
    }
    await this.#options.home.forgetForeignDeployment(domain);
  }

  /** Signs out of every other deployment on this device, as signing out at home does. */
  async signOutAll(): Promise<void> {
    this.#generation += 1;
    const entries = [...this.#entries.values()];
    for (const entry of entries) {
      entry.leaving = true;
      this.#close(entry);
    }
    await Promise.all(
      entries.map((entry) =>
        entry.client.logout().catch(() => {
          entry.client.forgetSession();
        }),
      ),
    );
  }

  /** Disconnects from every other deployment, keeping their sessions for next time. */
  stop(): void {
    this.#generation += 1;
    for (const entry of [...this.#entries.values()]) {
      entry.leaving = true;
      this.#close(entry);
    }
  }

  async #open(
    domain: string,
    inviteCode: string | undefined,
    generation: number,
  ): Promise<ForeignDeployment> {
    let entry = this.#entries.get(domain);
    if (entry === undefined) {
      const client = this.#options.client(domain);
      const created: Entry = {
        domain,
        status: "connecting",
        problem: null,
        client,
        sync: null,
        leaving: false,
        lastAcquiredAt: Number.NEGATIVE_INFINITY,
        unsubscribe: () => undefined,
      };
      created.unsubscribe = client.subscribe((session) => {
        if (session === null) {
          this.#sessionLost(created);
        }
      });
      this.#entries.set(domain, created);
      entry = created;
    } else {
      entry.status = "connecting";
      entry.problem = null;
    }
    this.#publish();
    try {
      const { protocol } = await entry.client.authMethods();
      if (commonVersion(CLIENT_PROTOCOL, protocol) === null) {
        entry.status = "incompatible";
        this.#publish();
        throw new IncompatibleDeploymentError(entry.domain);
      }
      if (entry.client.session === null) {
        await this.#acquire(entry, inviteCode);
      }
      if (generation !== this.#generation || entry.leaving) {
        return this.#view(entry);
      }
      entry.sync ??= this.#options.sync(entry.client);
      entry.sync.start();
      entry.status = "ready";
    } catch (e) {
      if (e instanceof IncompatibleDeploymentError) {
        throw e;
      }
      entry.status = "failed";
      entry.problem = e instanceof ApiProblemError ? e.message : String(e);
      this.#publish();
      throw e;
    }
    this.#publish();
    return this.#view(entry);
  }

  async #acquire(entry: Entry, inviteCode?: string): Promise<void> {
    entry.lastAcquiredAt = this.#now();
    const { assertion } = await this.#options.home.issueAssertion(entry.domain);
    await entry.client.signInWithAssertion(assertion, inviteCode);
  }

  /** A session abroad ended on its own: fetch another through the home, unless one just was. */
  #sessionLost(entry: Entry): void {
    if (entry.leaving || this.#entries.get(entry.domain) !== entry) {
      return;
    }
    entry.sync?.stop();
    entry.sync = null;
    if (this.#now() - entry.lastAcquiredAt < REACQUIRE_INTERVAL_MS) {
      entry.status = "failed";
      entry.problem ??= "signed out";
      this.#publish();
      return;
    }
    void this.#open(entry.domain, undefined, this.#generation).catch(() => undefined);
  }

  #close(entry: Entry): void {
    entry.unsubscribe();
    entry.sync?.stop();
    this.#entries.delete(entry.domain);
    this.#publish();
  }

  #view(entry: Entry): ForeignDeployment {
    return (
      this.#snapshot.find((view) => view.domain === entry.domain) ?? {
        domain: entry.domain,
        status: entry.status,
        problem: entry.problem,
        client: entry.client,
        sync: entry.sync,
      }
    );
  }

  #publish(): void {
    this.#snapshot = [...this.#entries.values()].map((entry) => ({
      domain: entry.domain,
      status: entry.status,
      problem: entry.problem,
      client: entry.client,
      sync: entry.sync,
    }));
    for (const listener of this.#listeners) {
      listener();
    }
  }
}
