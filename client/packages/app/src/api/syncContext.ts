import type { AspenSync } from "@aspen/protocol";
import { createContext } from "react";

/**
 * The sync of the deployment being shown: the home's, or another deployment's inside its
 * `ForeignScope`. Only valid inside `<SyncProvider>`.
 */
export const AspenSyncContext = createContext<AspenSync | null>(null);

/** The sync of the user's home deployment, wherever it is read. */
export const HomeSyncContext = createContext<AspenSync | null>(null);
