import { linkOptions, useNavigate } from "@tanstack/react-router";
import { channelLink, openInviteLink, useDomain, type Domain } from "@/features/messages/links";
import type { AspenLink } from "./aspenLinks";

/**
 * The route a code leads to: a sign-in code's screen (its id as `link`, since a route in a
 * fragment cannot carry a fragment of its own), the create-account screen, an invite's (one
 * that names no deployment on `domain`, the one shown), or a channel or DM on the deployment
 * its link is of. A registration invite is opened here only for the deployment in use: the
 * create-account screen is its own, so what takes one of another deployment decides what becomes
 * of it first (`routeJoinLink`).
 */
export function aspenLinkRoute(link: AspenLink, domain: Domain) {
  switch (link.kind) {
    case "deviceLink":
      return linkOptions({ to: "/device-link", search: { server: link.server, link: link.id } });
    case "registration":
      return linkOptions({ to: "/register", search: { invite: link.code } });
    case "invite":
      return openInviteLink(link.invite, domain);
    case "channel":
      return channelLink({ domain: link.domain, community: link.community }, link.channel);
    case "dm":
      return channelLink({ domain: link.domain, community: null }, link.channel);
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
