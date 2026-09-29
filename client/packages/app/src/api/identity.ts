import { identityOf as identityOn, type User } from "@aspen/protocol";
import { useContext, useEffect, useState } from "react";
import { authMethods } from "@/features/auth/authMethods";
import { HomeClientContext } from "./context";
import { ScopeDomainContext } from "./deploymentsContext";
import { useEverywhere, useSources } from "./everywhere";
import { AspenSyncContext } from "./syncContext";

/**
 * Who a user is across deployments (the protocol's `identityOf`). `deployment` is the domain of
 * the deployment the record is from, `home` the domain of the viewer's home, which stands for
 * it when it is the home's record.
 */
export function identityOf(
  user: Pick<User, "id" | "homeDomain" | "homeId">,
  deployment: string | null,
  home: string | null,
): string {
  return identityOn(user, deployment ?? home ?? "");
}

/** The domain of the viewer's home among deployments; `null` while unknown or when it has none. */
export function useHomeDomain(): string | null {
  return useHomeDomainState() ?? null;
}

/**
 * The domain of the deployment in scope: the one `ScopeDomainContext` names, or the viewer's
 * home. `null` while the home's is unknown, or when it takes no part in federation.
 */
export function useScopeDomain(): string | null {
  const scope = useContext(ScopeDomainContext);
  const home = useHomeDomain();
  return scope ?? home;
}

/**
 * The domain of the viewer's home among deployments: `undefined` while unknown, `null` when it
 * takes no part in federation.
 */
export function useHomeDomainState(): string | null | undefined {
  const home = useContext(HomeClientContext);
  const [domain, setDomain] = useState<string | null | undefined>(undefined);
  useEffect(() => {
    if (home === null) {
      return;
    }
    let current = true;
    authMethods(home).then(
      (methods) => {
        if (current) {
          setDomain(methods.federationDomain ?? null);
        }
      },
      () => undefined,
    );
    return () => {
      current = false;
    };
  }, [home]);
  return domain;
}

/**
 * Whether the viewer blocked `userId`, a user of the deployment in scope, on any deployment:
 * blocks are kept where they were made, and the client holds them across deployments by who
 * the person is (`identityOf`).
 */
export function useBlockedAnywhere(userId: string | undefined): boolean {
  const home = useHomeDomain();
  const scope = useContext(ScopeDomainContext);
  const sync = useContext(AspenSyncContext);
  const blocked = useBlockedIdentities();
  const user = userId === undefined ? undefined : sync?.store.user(userId);
  return user !== undefined && blocked.has(identityOf(user, scope, home));
}

/** Everyone the viewer blocked on any deployment they use, by `identityOf`. */
export function useBlockedIdentities(): ReadonlySet<string> {
  const home = useHomeDomain();
  return useEverywhere(["blocks"], (sources) => {
    const identities = new Set<string>();
    for (const source of sources) {
      for (const id of source.sync.store.blockedUsers()) {
        const user = source.sync.store.user(id);
        if (user !== undefined) {
          identities.add(identityOf(user, source.domain, home));
        }
      }
    }
    return identities;
  });
}

/**
 * Tells each deployment's sync who the viewer blocked everywhere, so that someone blocked on one
 * deployment is silenced and hidden in calls on every other (`AspenSync.setBlockedIdentities`).
 */
export function ShareBlocksAcrossDeployments() {
  const home = useHomeDomain();
  const identities = useBlockedIdentities();
  const sources = useSources();
  useEffect(() => {
    for (const source of sources) {
      source.sync.setBlockedIdentities(source.domain ?? home ?? "", identities);
    }
  }, [sources, home, identities]);
  return null;
}
