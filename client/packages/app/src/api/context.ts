import type { AspenClient, Session } from "@aspen/protocol";
import { createContext, useContext, useSyncExternalStore } from "react";

export const AspenClientContext = createContext<AspenClient | null>(null);

/** The client for the currently selected server. Only valid inside `<AspenProvider>`. */
export function useAspenClient(): AspenClient {
  const client = useContext(AspenClientContext);
  if (client === null) {
    throw new Error("useAspenClient must be used inside <AspenProvider>");
  }
  return client;
}

/** The current session, re-rendering on login, refresh, and logout. */
export function useSession(): Session | null {
  const client = useAspenClient();
  return useSyncExternalStore(client.subscribe, () => client.session);
}
