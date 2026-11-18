import type { Deployments, ForeignDeployment } from "@aspen/protocol";
import { useRouter } from "@tanstack/react-router";
import { createContext, useCallback, useContext, useSyncExternalStore } from "react";
import { useAccount, useAccountsHub } from "./accounts";
import { useHomeClient } from "./context";
import type { Source } from "./everywhere";
import { forgetPushAccounts } from "./pushBridge";

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
  const read = () => hub?.list ?? NONE;
  return useSyncExternalStore(hub?.subscribe ?? NOTHING_TO_HEAR, read, read);
}

/**
 * Ends this account's sign-ins on this device: at home, and first everywhere else it signed in
 * from there. `sources` are its deployments, whose phone subscriptions go with it.
 */
export function useEndSignIns(sources: readonly Source[]): () => Promise<void> {
  const hub = useContext(DeploymentsContext);
  const home = useHomeClient();
  return useCallback(async () => {
    const accounts = sources.flatMap(({ client }) =>
      client.session === null ? [] : [{ origin: client.baseUrl, userId: client.session.userId }],
    );
    await forgetPushAccounts(accounts).catch(() => undefined);
    await hub?.signOutAll().catch(() => undefined);
    await home.logout();
  }, [hub, home, sources]);
}

/**
 * Signs `id` out of this device, this scope's account unless another is named. Other accounts
 * held here stay signed in, and the window moves to whichever is then in front.
 */
export function useSignOut(): (id?: string) => Promise<void> {
  const accounts = useAccountsHub();
  const own = useAccount().id;
  const router = useRouter();
  return useCallback(
    async (id = own) => {
      const front = accounts.view().front;
      await accounts.signOut(id);
      const now = accounts.view().front;
      if (now !== front) {
        router.history.push(accounts.routeOf(now) ?? "/");
      }
    },
    [accounts, own, router],
  );
}
