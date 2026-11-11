import { useContext } from "react";
import { HomeClientContext } from "@/api/context";
import { useForeignDeployments } from "@/api/deploymentsContext";
import type { KnownHosts } from "@/features/messages/selfLinks";

/**
 * The home deployment's own addresses, which a link to it may be on: the page's own, and the
 * address the app reaches the deployment at, where it has one chosen.
 */
export function useHomeHosts(): readonly string[] {
  const home = useContext(HomeClientContext);
  return [window.location.host, ...(home === null ? [] : [new URL(home.baseUrl).host])];
}

/**
 * The addresses of the deployments the user uses: the home's (`useHomeHosts`) and every other
 * one they sign in to from there.
 */
export function useKnownHosts(): KnownHosts {
  const home = useHomeHosts();
  const foreign = useForeignDeployments();
  return { home, foreign: foreign.map((d) => d.domain) };
}
