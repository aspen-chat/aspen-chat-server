import { linkOptions, useNavigate } from "@tanstack/react-router";
import { openInviteLink, useDomain, type Domain } from "@/features/messages/links";
import type { AspenLink } from "./aspenLinks";

/**
 * The route a code leads to: a sign-in code's screen (its id as `link`, since a route in a
 * fragment cannot carry a fragment of its own), or an invite's, a bare invite on `domain`.
 */
export function aspenLinkRoute(link: AspenLink, domain: Domain) {
  switch (link.kind) {
    case "deviceLink":
      return linkOptions({ to: "/device-link", search: { server: link.server, link: link.id } });
    case "registration":
      return linkOptions({ to: "/register", search: { invite: link.code } });
    case "invite":
      return openInviteLink(link.invite, domain);
  }
}

/** Opens what a scanned code leads to, from the deployment shown. */
export function useOpenAspenLink(): (link: AspenLink) => void {
  const navigate = useNavigate();
  const domain = useDomain();
  return (link) => {
    void navigate(aspenLinkRoute(link, domain));
  };
}
