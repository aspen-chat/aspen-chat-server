import { linkOptions, useParams } from "@tanstack/react-router";

/**
 * The deployment a route belongs to: `null` for the user's home, whose routes are at the root,
 * or another deployment's domain, whose routes are under `/at/{domain}`.
 */
export type Domain = string | null;

/**
 * Where a channel's routes hang: its deployment, and the id of its community, or `null` for a
 * DM or group DM, whose routes are under `/dms`. A thread's routes hang where its parent
 * channel's do.
 */
export interface ChannelHome {
  readonly domain: Domain;
  readonly community: string | null;
}

/** The deployment of the route being shown. */
export function useDomain(): Domain {
  return useParams({ strict: false }).domain ?? null;
}

/** The deployment's front page: its first community, or somewhere to start. */
export function deploymentLink(domain: Domain) {
  return domain === null
    ? linkOptions({ to: "/" })
    : linkOptions({ to: "/at/$domain", params: { domain } });
}

/** The route of a community's channel list. */
export function communityLink(domain: Domain, communityId: string) {
  return domain === null
    ? linkOptions({ to: "/communities/$communityId", params: { communityId } })
    : linkOptions({
        to: "/at/$domain/communities/$communityId",
        params: { domain, communityId },
      });
}

/** The route of the DM list. */
export function dmsLink(domain: Domain) {
  return domain === null
    ? linkOptions({ to: "/dms" })
    : linkOptions({ to: "/at/$domain/dms", params: { domain } });
}

/** The route of an invite. */
export function inviteLink(domain: Domain, code: string) {
  return domain === null
    ? linkOptions({ to: "/invite/$code", params: { code } })
    : linkOptions({ to: "/at/$domain/invite/$code", params: { domain, code } });
}

/** The route of a channel's history. */
export function channelLink(home: ChannelHome, channelId: string) {
  const { domain, community } = home;
  if (community === null) {
    return domain === null
      ? linkOptions({ to: "/dms/$channelId", params: { channelId } })
      : linkOptions({ to: "/at/$domain/dms/$channelId", params: { domain, channelId } });
  }
  return domain === null
    ? linkOptions({
        to: "/communities/$communityId/channels/$channelId",
        params: { communityId: community, channelId },
      })
    : linkOptions({
        to: "/at/$domain/communities/$communityId/channels/$channelId",
        params: { domain, communityId: community, channelId },
      });
}

/** The route of a channel's history opened around one message. */
export function messageLink(home: ChannelHome, channelId: string, messageId: string) {
  const { domain, community } = home;
  if (community === null) {
    return domain === null
      ? linkOptions({
          to: "/dms/$channelId/messages/$messageId",
          params: { channelId, messageId },
        })
      : linkOptions({
          to: "/at/$domain/dms/$channelId/messages/$messageId",
          params: { domain, channelId, messageId },
        });
  }
  return domain === null
    ? linkOptions({
        to: "/communities/$communityId/channels/$channelId/messages/$messageId",
        params: { communityId: community, channelId, messageId },
      })
    : linkOptions({
        to: "/at/$domain/communities/$communityId/channels/$channelId/messages/$messageId",
        params: { domain, communityId: community, channelId, messageId },
      });
}

/** The route of a channel with one of its threads open beside it. */
export function threadLink(home: ChannelHome, channelId: string, threadId: string) {
  const { domain, community } = home;
  if (community === null) {
    return domain === null
      ? linkOptions({
          to: "/dms/$channelId/threads/$threadId",
          params: { channelId, threadId },
        })
      : linkOptions({
          to: "/at/$domain/dms/$channelId/threads/$threadId",
          params: { domain, channelId, threadId },
        });
  }
  return domain === null
    ? linkOptions({
        to: "/communities/$communityId/channels/$channelId/threads/$threadId",
        params: { communityId: community, channelId, threadId },
      })
    : linkOptions({
        to: "/at/$domain/communities/$communityId/channels/$channelId/threads/$threadId",
        params: { domain, communityId: community, channelId, threadId },
      });
}
