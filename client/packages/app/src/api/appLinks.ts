import { useEffect, useRef } from "react";
import { detectShell } from "@/config";
import { parseAspenLink, type AspenLink } from "@/features/qr/aspenLinks";
import { APP_LINK_BASE } from "@/features/qr/shareLinks";
import { aspenLinkRoute } from "@/features/qr/openLink";
import { router } from "@/router";

/**
 * In the mobile apps, opens the `aspen://app/…` links that invites and sign-in codes are shared as
 * when the deployment names no web client (`shareUrl`), whether the system hands one over while
 * the app runs or launches the app with it. `onServer` hears a sign-in code's server, for an app
 * that has none chosen yet. Other `aspen:` addresses (the passkey hand-off's return) are left to
 * whatever is waiting for them.
 */
export function useOpenAppLinks(onServer: (link: AspenLink & { kind: "deviceLink" }) => void) {
  const latest = useRef(onServer);
  useEffect(() => {
    latest.current = onServer;
  });
  useEffect(() => {
    if (detectShell() !== "mobile") {
      return;
    }
    const open = (url: string) => {
      if (!url.startsWith(APP_LINK_BASE + "/")) {
        return;
      }
      const link = parseAspenLink(url);
      if (link === null) {
        return;
      }
      if (link.kind === "deviceLink") {
        latest.current(link);
      }
      void router.navigate(aspenLinkRoute(link, null));
    };
    let removed = false;
    let remove: (() => void) | null = null;
    void import("@capacitor/app").then(async ({ App }) => {
      const launched = await App.getLaunchUrl();
      if (launched !== undefined && !removed) {
        open(launched.url);
      }
      const handle = await App.addListener("appUrlOpen", ({ url }) => {
        open(url);
      });
      if (removed) {
        void handle.remove();
      } else {
        remove = () => void handle.remove();
      }
    });
    return () => {
      removed = true;
      remove?.();
    };
  }, []);
}
