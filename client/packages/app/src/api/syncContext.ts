import type { AspenSync } from "@aspen/protocol";
import { createContext } from "react";

/** The sync instance for the signed-in session. Only valid inside `<SyncProvider>`. */
export const AspenSyncContext = createContext<AspenSync | null>(null);
