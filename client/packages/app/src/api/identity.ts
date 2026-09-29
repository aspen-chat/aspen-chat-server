import type { User } from "@aspen/protocol";
import { useContext, useEffect, useState } from "react";
import { authMethods } from "@/features/auth/authMethods";
import { HomeClientContext } from "./context";
import { ScopeDomainContext } from "./deploymentsContext";
import { useEverywhere } from "./everywhere";
import { AspenSyncContext } from "./syncContext";

/**
 * Who a user is across deployments: their home's domain and their id there. On their own
 * deployment that is the deployment's domain and their id; elsewhere their record names both
 * (`homeDomain`, `homeId`). `deployment` is the domain of the deployment the record is from,
 * `home` the domain of the viewer's home, which stands for it when it is the home's record.
 */
export function identityOf(
  user: Pick<User, "id" | "homeDomain" | "homeId">,
  deployment: string | null,
  home: string | null,
): string {
  if (user.homeDomain != null && user.homeId != null) {
    return `${user.homeDomain}/${user.homeId}`;
  }
  return `${deployment ?? home ?? ""}/${user.id}`;
}

/** The domain of the viewer's home among deployments; `null` while unknown or when it has none. */
export function useHomeDomain(): string | null {
  const home = useContext(HomeClientContext);
  const [domain, setDomain] = useState<string | null>(null);
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
  const blocked = useEverywhere(["blocks"], (sources) => {
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
  const user = userId === undefined ? undefined : sync?.store.user(userId);
  return user !== undefined && blocked.has(identityOf(user, scope, home));
}
