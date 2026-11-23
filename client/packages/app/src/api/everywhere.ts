import type { AspenSync, Topic } from "@aspen/protocol";
import { useCallback, useContext, useMemo, useRef, useSyncExternalStore } from "react";
import type { AspenClient } from "@aspen/protocol";
import { useAccount } from "./accounts";
import { HomeClientContext } from "./context";
import { useForeignDeployments } from "./deploymentsContext";
import { HomeSyncContext } from "./syncContext";

/** One deployment an account is signed in to: `domain` is `null` for the account's home. */
export interface Source {
  readonly accountId: string;
  readonly domain: string | null;
  readonly client: AspenClient;
  readonly sync: AspenSync;
  readonly homeClient: AspenClient;
  readonly homeSync: AspenSync;
}

/** The user's home and every other deployment they are signed in to, home first. */
export function useSources(): readonly Source[] {
  const home = useContext(HomeSyncContext);
  const homeClient = useContext(HomeClientContext);
  const foreign = useForeignDeployments();
  const accountId = useAccount().id;
  return useMemo(() => {
    if (home === null || homeClient === null) {
      return [];
    }
    const of = { accountId, homeClient, homeSync: home };
    const sources: Source[] = [{ ...of, domain: null, client: homeClient, sync: home }];
    for (const entry of foreign) {
      if (entry.status === "ready" && entry.sync !== null) {
        sources.push({ ...of, domain: entry.domain, client: entry.client, sync: entry.sync });
      }
    }
    return sources;
  }, [accountId, home, homeClient, foreign]);
}

/**
 * Reads across every deployment, as the lists that mix them do: `read` runs again whenever one
 * of `topics` changes in any of their stores, and its answer is kept until then.
 */
export function useEverywhere<T>(
  topics: readonly Topic[],
  read: (sources: readonly Source[]) => T,
): T {
  return useAcross(useSources(), topics, read);
}

/** As `useEverywhere`, over the sources given. */
export function useAcross<T>(
  sources: readonly Source[],
  topics: readonly Topic[],
  read: (sources: readonly Source[]) => T,
): T {
  const version = useRef(0);
  const topicKey = topics.join("\u0000");
  const subscribe = useCallback(
    (listener: () => void) => {
      const stops = sources.flatMap((source) =>
        topicKey.split("\u0000").map((topic) =>
          source.sync.store.subscribe(topic, () => {
            version.current += 1;
            listener();
          }),
        ),
      );
      return () => {
        for (const stop of stops) {
          stop();
        }
      };
    },
    [sources, topicKey],
  );
  const seen = useSyncExternalStore(subscribe, () => version.current);
  // `read` is the caller's and may be a new function each render; the answer follows the stores.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useMemo(() => read(sources), [sources, seen]);
}
