import type { AspenClient } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useHomeClient } from "@/api/context";
import { detectShell, type Shell } from "@/config";
import { router } from "@/router";

/** The scheme the desktop and mobile apps open, for links when no web client is known. */
export const APP_LINK_BASE = "aspen://app";

/**
 * The address a link to share names for `path` (a route, with its query and fragment): under the
 * deployment's web client (`webClientUrl`, its `[web_client] url`), which opens on any device; or,
 * where the deployment names none, the web client serving this page; or, in the desktop and
 * mobile apps, an `aspen:` link, which opens only where Aspen is installed.
 */
export function shareUrl(
  path: string,
  webClientUrl: string | null,
  shell: Shell = detectShell(),
): string {
  if (webClientUrl !== null) {
    return webClientUrl + path;
  }
  if (shell === "web") {
    return new URL(router.history.createHref(path), window.location.href).toString();
  }
  return APP_LINK_BASE + path;
}

/** Each home client's web client address, read once. */
const webClientUrls = new WeakMap<AspenClient, Promise<string | null>>();

function webClientUrlOf(client: AspenClient): Promise<string | null> {
  let known = webClientUrls.get(client);
  if (known === undefined) {
    known = client.deploymentProfile().then(
      (profile) => profile.webClientUrl ?? null,
      () => null,
    );
    webClientUrls.set(client, known);
  }
  return known;
}

/**
 * Makes links to share (`shareUrl`) once the home deployment has said where its web client is;
 * `null` until then, so a link or QR code is never shown with an address that is about to change.
 */
export function useShareUrl(): ((path: string) => string) | null {
  const client = useHomeClient();
  const [known, setKnown] = useState<{ client: AspenClient; url: string | null } | null>(null);
  useEffect(() => {
    let live = true;
    void webClientUrlOf(client).then((url) => {
      if (live) {
        setKnown({ client, url });
      }
    });
    return () => {
      live = false;
    };
  }, [client]);
  if (known?.client !== client) {
    return null;
  }
  return (path) => shareUrl(path, known.url);
}
