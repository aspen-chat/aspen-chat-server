import {
  offeredProviders,
  requireSecure,
  type AspenClient,
  type IdentityProviderOutcome,
  type IdentityProviderPurpose,
  type OfferedIdentityProvider,
  type PasskeyHandoff,
} from "@aspen/protocol";
import { useCallback } from "react";
import { useAspenClient } from "@/api/context";
import { detectShell, type Shell } from "@/config";
import { useAuthMethods } from "@/features/auth/authMethods";
import { mobileHandoff } from "@/features/auth/passkeyTransport";
import { useMessages } from "@/i18n/context";

/** A provider to run a flow at: its id, and its name for what the screens say of it. */
export type ProviderChoice = Pick<OfferedIdentityProvider, "id" | "name">;

/**
 * A flow this tab sent to its provider, kept in `sessionStorage` (`aspen.identityProviderFlow.
 * {flow id}`) until the browser comes back to `/sign-in/provider`: the verifier never leaves
 * this tab, and the page the person was on is where they land.
 */
export interface StartedFlow {
  verifier: string;
  purpose: IdentityProviderPurpose;
  providerName: string;
  /** The path, query, and fragment the tab was at when the flow started. */
  returnPath: string;
  /** Milliseconds since the epoch, for dropping flows long abandoned. */
  startedAt: number;
}

const FLOW_KEY = "aspen.identityProviderFlow.";

/**
 * How long a started flow is worth keeping: the server keeps a flow ten minutes, and its outcome
 * five more for the claim.
 */
export const FLOW_KEPT_MS = 15 * 60 * 1000;

/**
 * Keeps a started flow for the return page, dropping any this tab abandoned at their provider
 * long enough ago that the server has forgotten them too. `false` when this browser keeps
 * nothing (storage turned off), and the flow could not be claimed on return.
 */
export function keepFlow(
  id: string,
  flow: StartedFlow,
  storage: Storage = sessionStorage,
): boolean {
  try {
    for (let i = storage.length - 1; i >= 0; i--) {
      const key = storage.key(i);
      if (key?.startsWith(FLOW_KEY) === true) {
        const kept = readFlow(storage.getItem(key));
        if (kept === null || flow.startedAt - kept.startedAt > FLOW_KEPT_MS) {
          storage.removeItem(key);
        }
      }
    }
    storage.setItem(FLOW_KEY + id, JSON.stringify(flow));
    return true;
  } catch {
    return false;
  }
}

/** Takes a flow this tab started, once: `null` when it started elsewhere or was taken. */
export function takeFlow(id: string, storage: Storage = sessionStorage): StartedFlow | null {
  try {
    const flow = readFlow(storage.getItem(FLOW_KEY + id));
    storage.removeItem(FLOW_KEY + id);
    return flow;
  } catch {
    return null;
  }
}

function readFlow(text: string | null): StartedFlow | null {
  if (text === null) {
    return null;
  }
  try {
    const value = JSON.parse(text) as Partial<StartedFlow> | null;
    return typeof value?.verifier === "string" &&
      typeof value.purpose === "string" &&
      typeof value.providerName === "string" &&
      typeof value.returnPath === "string" &&
      typeof value.startedAt === "number"
      ? (value as StartedFlow)
      : null;
  } catch {
    return null;
  }
}

/**
 * How a shell's flows reach the system browser: the desktop and mobile apps through a passkey
 * hand-off's return address (a loopback port, or `aspen://auth/passkey`), opening the provider
 * where the passkey page would be; `null` for the web client, which sends its own tab.
 */
export function providerHandoff(shell: Shell, server: string): PasskeyHandoff | null {
  switch (shell) {
    case "desktop": {
      const bridge = window.aspenDesktop?.passkeyHandoff;
      if (bridge === undefined) {
        return null;
      }
      return {
        async prepare() {
          const { id, returnTo } = await bridge.prepare();
          return {
            returnTo,
            open: (url) => bridge.openProvider(id, url, server),
            dispose: () => {
              bridge.dispose(id);
            },
          };
        },
      };
    }
    case "mobile":
      return mobileHandoff;
    case "web":
      return null;
  }
}

/** The providers the current server offers; empty while unknown and where it offers none. */
export function useIdentityProviders(): OfferedIdentityProvider[] {
  const methods = useAuthMethods();
  return methods === null ? [] : offeredProviders(methods);
}

/**
 * Runs a flow at a provider. In the apps it goes through the system browser and resolves to
 * what it did. The web client sends its own tab to the provider, keeping the flow for the
 * return page (`ProviderReturn`), and never resolves, since the page is leaving. Throws as
 * `AspenClient.runIdentityProviderHandoff` does.
 */
export function useProviderFlow(): (
  provider: ProviderChoice,
  purpose: IdentityProviderPurpose,
) => Promise<IdentityProviderOutcome> {
  const client = useAspenClient();
  const m = useMessages();
  return useCallback(
    (provider, purpose) =>
      runProviderFlow(client, provider, purpose, m.identityProviders.noStorage),
    [client, m],
  );
}

async function runProviderFlow(
  client: AspenClient,
  provider: ProviderChoice,
  purpose: IdentityProviderPurpose,
  noStorage: string,
): Promise<IdentityProviderOutcome> {
  const handoff = providerHandoff(detectShell(), client.baseUrl);
  if (handoff !== null) {
    return client.runIdentityProviderHandoff(provider.id, purpose, handoff);
  }
  const { flow, verifier } = await client.startIdentityProviderFlow(provider.id, purpose);
  // The deployment names the provider's page; this tab goes there only over HTTPS, whatever
  // scheme it names.
  const authorizeUrl = requireSecure(flow.authorizeUrl);
  const { pathname, search, hash } = window.location;
  const kept = keepFlow(flow.id, {
    verifier,
    purpose,
    providerName: provider.name,
    returnPath: pathname + search + hash,
    startedAt: Date.now(),
  });
  if (!kept) {
    throw new Error(noStorage);
  }
  window.location.assign(authorizeUrl);
  return new Promise<never>(() => undefined);
}
