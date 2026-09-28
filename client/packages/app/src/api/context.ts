import type { AspenClient, Session } from "@aspen/protocol";
import { createContext, useContext, useSyncExternalStore } from "react";

/**
 * The client of the deployment being shown: the home's, or another deployment's inside its
 * `ForeignScope`.
 */
export const AspenClientContext = createContext<AspenClient | null>(null);

/** The client of the user's home deployment, the server they chose, wherever it is read. */
export const HomeClientContext = createContext<AspenClient | null>(null);

/** The client of the deployment being shown. Only valid inside `<AspenProvider>`. */
export function useAspenClient(): AspenClient {
  const client = useContext(AspenClientContext);
  if (client === null) {
    throw new Error("useAspenClient must be used inside <AspenProvider>");
  }
  return client;
}

/** The client of the user's home deployment. Only valid inside `<AspenProvider>`. */
export function useHomeClient(): AspenClient {
  const client = useContext(HomeClientContext);
  if (client === null) {
    throw new Error("useHomeClient must be used inside <AspenProvider>");
  }
  return client;
}

/** The current session, re-rendering on login, refresh, and logout. */
export function useSession(): Session | null {
  const client = useAspenClient();
  return useSyncExternalStore(client.subscribe, () => client.session);
}
