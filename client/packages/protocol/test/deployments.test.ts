import { describe, expect, it, vi } from "vitest";
import {
  Deployments,
  IncompatibleDeploymentError,
  REACQUIRE_INTERVAL_MS,
} from "../src/deployments";
import type { AspenClient } from "../src/http";
import { ApiProblemError } from "../src/problem";
import type { Session } from "../src/session";
import type { AspenSync } from "../src/sync";

const session: Session = {
  userId: "u",
  refreshToken: "r",
  sessionToken: "s",
  sessionTokenExpires: "2099-01-01T00:00:00Z",
};

/** A client of another deployment: a session that tests can end, and sign-in by assertion. */
function foreignClient(signedIn = false, protocol = { version: 1, minimum: 1 }) {
  const listeners = new Set<(s: Session | null) => void>();
  const client = {
    session: signedIn ? session : null,
    authMethods: vi.fn(() => Promise.resolve({ protocol })),
    subscribe: (listener: (s: Session | null) => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    signInWithAssertion: vi.fn((assertion: string) => {
      client.session = { ...session, sessionToken: assertion };
      return Promise.resolve(client.session);
    }),
    logout: vi.fn(() => {
      client.end();
      return Promise.resolve();
    }),
    forgetSession: vi.fn(() => {
      client.end();
    }),
    end() {
      client.session = null;
      for (const listener of listeners) {
        listener(null);
      }
    },
  };
  return client;
}

function fakeSync() {
  return { start: vi.fn(), stop: vi.fn() };
}

function world(
  options: { listed?: string[]; refuse?: string; signedIn?: string[]; future?: string } = {},
) {
  let assertions = 0;
  const home = {
    foreignDeployments: vi.fn(() =>
      Promise.resolve((options.listed ?? []).map((domain) => ({ domain }))),
    ),
    issueAssertion: vi.fn((audience: string) => {
      if (audience === options.refuse) {
        return Promise.reject(
          new ApiProblemError({
            code: "federationRefused",
            title: "Federation does not allow this.",
            status: 403,
          }),
        );
      }
      assertions += 1;
      return Promise.resolve({ assertion: `a${String(assertions)}`, audience, expiresAt: "" });
    }),
    forgetForeignDeployment: vi.fn(() => Promise.resolve()),
  };
  const clients = new Map<string, ReturnType<typeof foreignClient>>();
  const syncs: ReturnType<typeof fakeSync>[] = [];
  let now = 0;
  const deployments = new Deployments({
    home: home as unknown as AspenClient,
    client: (domain) => {
      const client = foreignClient(
        options.signedIn?.includes(domain) ?? false,
        domain === options.future ? { version: 9, minimum: 7 } : { version: 1, minimum: 1 },
      );
      clients.set(domain, client);
      return client as unknown as AspenClient;
    },
    sync: () => {
      const sync = fakeSync();
      syncs.push(sync);
      return sync as unknown as AspenSync;
    },
    now: () => now,
  });
  return {
    home,
    clients,
    syncs,
    deployments,
    advance: (ms: number) => {
      now += ms;
    },
  };
}

describe("Deployments", () => {
  it("signs in to every deployment the home lists, by assertion where there is no session", async () => {
    const { deployments, clients, home, syncs } = world({
      listed: ["b.example", "c.example"],
      signedIn: ["c.example"],
    });
    await deployments.start();
    expect(deployments.list.map((d) => [d.domain, d.status])).toEqual([
      ["b.example", "ready"],
      ["c.example", "ready"],
    ]);
    expect(home.issueAssertion).toHaveBeenCalledTimes(1);
    expect(clients.get("b.example")?.signInWithAssertion).toHaveBeenCalledWith("a1", undefined);
    expect(syncs.every((sync) => sync.start.mock.calls.length === 1)).toBe(true);
  });

  it("joins a new deployment and says why one refuses", async () => {
    const { deployments } = world({ refuse: "closed.example" });
    const joined = await deployments.join("b.example", "invite");
    expect(joined.status).toBe("ready");
    await expect(deployments.join("closed.example")).rejects.toThrow(ApiProblemError);
    expect(deployments.get("closed.example")).toMatchObject({
      status: "failed",
      problem: "Federation does not allow this.",
    });
  });

  it("signs in nowhere that speaks no protocol version this client does", async () => {
    const { deployments, home } = world({ future: "future.example" });
    await expect(deployments.join("future.example")).rejects.toThrow(IncompatibleDeploymentError);
    expect(deployments.get("future.example")?.status).toBe("incompatible");
    expect(home.issueAssertion).not.toHaveBeenCalled();
  });

  it("replaces a lost session through the home, but not again at once", async () => {
    const { deployments, clients, home, advance } = world({ listed: ["b.example"] });
    await deployments.start();
    advance(REACQUIRE_INTERVAL_MS);
    clients.get("b.example")?.end();
    await vi.waitFor(() => {
      expect(deployments.get("b.example")?.status).toBe("ready");
    });
    expect(home.issueAssertion).toHaveBeenCalledTimes(2);
    clients.get("b.example")?.end();
    expect(deployments.get("b.example")?.status).toBe("failed");
    expect(home.issueAssertion).toHaveBeenCalledTimes(2);
  });

  it("leaving signs out there and has the home forget it", async () => {
    const { deployments, clients, home, syncs } = world({ listed: ["b.example"] });
    await deployments.start();
    await deployments.leave("b.example");
    expect(clients.get("b.example")?.logout).toHaveBeenCalled();
    expect(home.forgetForeignDeployment).toHaveBeenCalledWith("b.example");
    expect(syncs[0]?.stop).toHaveBeenCalled();
    expect(deployments.list).toEqual([]);
    expect(home.issueAssertion).toHaveBeenCalledTimes(1);
  });

  it("signing out everywhere signs out of each and replaces none", async () => {
    const { deployments, clients, home } = world({ listed: ["b.example", "c.example"] });
    await deployments.start();
    await deployments.signOutAll();
    expect([...clients.values()].every((c) => c.logout.mock.calls.length === 1)).toBe(true);
    expect(deployments.list).toEqual([]);
    expect(home.issueAssertion).toHaveBeenCalledTimes(2);
    expect(home.forgetForeignDeployment).not.toHaveBeenCalled();
  });
});
