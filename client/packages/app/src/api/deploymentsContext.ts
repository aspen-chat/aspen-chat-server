import type { Deployments, ForeignDeployment } from "@aspen/protocol";
import { createContext, useCallback, useContext, useSyncExternalStore } from "react";
import { useHomeClient } from "./context";

/**
 * The deployment whose client and sync are in scope: `null` for the user's home, or another
 * deployment's domain, as `ForeignScope`, `HomeScope`, and `SourceScope` set it.
 */
export const ScopeDomainContext = createContext<string | null>(null);

export const DeploymentsContext = createContext<Deployments | null>(null);

/** The other deployments the user signs in to from home. Only valid inside the signed-in app. */
export function useDeploymentsHub(): Deployments {
  const hub = useContext(DeploymentsContext);
  if (hub === null) {
    throw new Error("useDeploymentsHub must be used inside <DeploymentsProvider>");
  }
  return hub;
}

const NONE: readonly ForeignDeployment[] = [];
const NOTHING_TO_HEAR = () => () => undefined;

/**
 * Every other deployment, re-rendering as they are signed in to, left, or fail; none outside
 * the signed-in app.
 */
export function useForeignDeployments(): readonly ForeignDeployment[] {
  const hub = useContext(DeploymentsContext);
  return useSyncExternalStore(hub?.subscribe ?? NOTHING_TO_HEAR, () => hub?.list ?? NONE);
}

/**
 * Signs out at home, and first everywhere else the user signed in from there on this device.
 */
export function useSignOut(): () => Promise<void> {
  const hub = useContext(DeploymentsContext);
  const home = useHomeClient();
  return useCallback(async () => {
    await hub?.signOutAll().catch(() => undefined);
    await home.logout();
  }, [hub, home]);
}
