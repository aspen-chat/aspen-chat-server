import { AspenClient } from "@aspen/protocol";
import { useMemo, type ReactNode } from "react";
import { AspenClientContext, HomeClientContext } from "./context";
import { clockStore, deviceStore, solveInWorkers } from "./devices";
import { sessionStore } from "./sessions";

/**
 * Owns the `AspenClient` for the chosen server, the user's home. A new server address means a
 * new client.
 */
export function AspenProvider({ serverUrl, children }: { serverUrl: string; children: ReactNode }) {
  const client = useMemo(
    () =>
      new AspenClient({
        baseUrl: serverUrl,
        sessionStore: sessionStore(),
        deviceStore: deviceStore(serverUrl),
        clockStore: clockStore(serverUrl),
        solvePuzzle: solveInWorkers,
      }),
    [serverUrl],
  );
  return (
    <HomeClientContext.Provider value={client}>
      <AspenClientContext.Provider value={client}>{children}</AspenClientContext.Provider>
    </HomeClientContext.Provider>
  );
}
