import {
  AspenClient,
  AspenSync,
  Deployments,
  WebStorageSessionStore,
  deploymentUrl,
} from "@aspen/protocol";
import { ArrowClockwiseIcon, GlobeIcon } from "@phosphor-icons/react";
import {
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { Button } from "react-aria-components";
import { reportActivity } from "./activity";
import type { Source } from "./everywhere";
import { AspenClientContext, useHomeClient } from "./context";
import { DeploymentsContext, useDeploymentsHub, useForeignDeployments } from "./deploymentsContext";
import { AspenSyncContext, HomeSyncContext } from "./syncContext";
import { primaryButtonClass } from "@/features/auth/styles";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Where a session for another deployment is kept: one key per home account and deployment, so
 * an account signed in later on the same browser never picks up another's sessions.
 */
function sessionKey(homeUser: string, domain: string): string {
  return `aspen.session.${homeUser}@${domain}`;
}

/**
 * Owns the user's other deployments for as long as they are signed in at home: signs in to each
 * the home lists, gives each a sync sharing the home's preferences, reports the user's activity
 * to each so they show as present there too, and disconnects on unmount.
 */
export function DeploymentsProvider({
  homeSync,
  children,
}: {
  homeSync: AspenSync;
  children: ReactNode;
}) {
  const home = useHomeClient();
  const hub = useMemo(
    () =>
      new Deployments({
        home,
        client: (domain) =>
          new AspenClient({
            baseUrl: deploymentUrl(domain),
            sessionStore: new WebStorageSessionStore(
              window.localStorage,
              sessionKey(home.session?.userId ?? "", domain),
            ),
          }),
        sync: (client) =>
          new AspenSync({
            client,
            preferences: homeSync.preferences,
            validateEvents: import.meta.env.DEV,
            onInvalidEvent: (raw, errors) => {
              console.warn("dropped a server event that does not match the schema", errors, raw);
            },
          }),
      }),
    [home, homeSync],
  );
  useEffect(() => {
    void hub.start();
    return () => {
      hub.stop();
    };
  }, [hub]);
  const list = useSyncExternalStore(hub.subscribe, () => hub.list);
  useEffect(() => {
    const stops = list.flatMap((entry) =>
      entry.sync === null ? [] : [reportActivity(entry.sync)],
    );
    return () => {
      for (const stop of stops) {
        stop();
      }
    };
  }, [list]);
  return <DeploymentsContext.Provider value={hub}>{children}</DeploymentsContext.Provider>;
}

/** Runs `children` against another deployment, signing in there first when needed. */
export function ForeignScope({ domain, children }: { domain: string; children: ReactNode }) {
  const m = useMessages();
  const hub = useDeploymentsHub();
  const entry = useForeignDeployments().find((d) => d.domain === domain);
  const [joining, setJoining] = useState(false);
  if (entry?.status === "ready" && entry.sync !== null) {
    return (
      <AspenClientContext.Provider value={entry.client}>
        <AspenSyncContext.Provider value={entry.sync}>{children}</AspenSyncContext.Provider>
      </AspenClientContext.Provider>
    );
  }
  const join = () => {
    setJoining(true);
    hub
      .join(domain)
      .catch(() => undefined)
      .finally(() => {
        setJoining(false);
      });
  };
  return (
    <main className="flex min-w-0 flex-1 items-center justify-center bg-surface p-6">
      <section
        aria-labelledby="foreign-heading"
        className="flex max-w-md flex-col items-center gap-3 text-center"
      >
        <GlobeIcon size={32} aria-hidden="true" className="text-ink-muted" />
        <h1 id="foreign-heading" className="text-lg font-semibold break-words">
          {domain}
        </h1>
        {entry === undefined && !joining ? (
          <>
            <p className="text-sm text-ink-muted">
              {format(m.deployments.notSignedIn, { domain })}
            </p>
            <Button onPress={join} className={primaryButtonClass}>
              {m.deployments.signIn}
            </Button>
          </>
        ) : entry?.status === "incompatible" ? (
          <p role="alert" className="text-sm text-danger">
            {format(m.deployments.incompatible, { domain })}
          </p>
        ) : entry?.status === "failed" && !joining ? (
          <>
            <p role="alert" className="text-sm text-danger">
              {format(m.deployments.failed, { domain, detail: entry.problem ?? "" })}
            </p>
            <Button onPress={join} className={primaryButtonClass}>
              <ArrowClockwiseIcon size={16} aria-hidden="true" />
              {m.deployments.retry}
            </Button>
          </>
        ) : (
          <p role="status" className="text-sm text-ink-muted">
            {format(m.deployments.signingIn, { domain })}
          </p>
        )}
      </section>
    </main>
  );
}

/**
 * Runs `children` against the user's home deployment wherever they are shown: their account,
 * settings, and security are home's even while another deployment's routes are open.
 */
export function HomeScope({ children }: { children: ReactNode }) {
  const client = useHomeClient();
  const sync = useContext(HomeSyncContext);
  return (
    <AspenClientContext.Provider value={client}>
      <AspenSyncContext.Provider value={sync}>{children}</AspenSyncContext.Provider>
    </AspenClientContext.Provider>
  );
}

/**
 * Runs `children` against one deployment, as a list that mixes deployments does for each of
 * its entries, so what an entry shows and does is that deployment's.
 */
export function SourceScope({ source, children }: { source: Source; children: ReactNode }) {
  return (
    <AspenClientContext.Provider value={source.client}>
      <AspenSyncContext.Provider value={source.sync}>{children}</AspenSyncContext.Provider>
    </AspenClientContext.Provider>
  );
}
