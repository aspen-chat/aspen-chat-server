import { linkOptions } from "@tanstack/react-router";

/**
 * Where a channel's routes hang: the id of its community, or `null` for a DM or group DM,
 * whose routes are under `/dms`. A thread's routes hang where its parent channel's do.
 */
export type ChannelHome = string | null;

/** The route of a channel's history. */
export function channelLink(home: ChannelHome, channelId: string) {
  return home === null
    ? linkOptions({ to: "/dms/$channelId", params: { channelId } })
    : linkOptions({
        to: "/communities/$communityId/channels/$channelId",
        params: { communityId: home, channelId },
      });
}

/** The route of a channel's history opened around one message. */
export function messageLink(home: ChannelHome, channelId: string, messageId: string) {
  return home === null
    ? linkOptions({ to: "/dms/$channelId/messages/$messageId", params: { channelId, messageId } })
    : linkOptions({
        to: "/communities/$communityId/channels/$channelId/messages/$messageId",
        params: { communityId: home, channelId, messageId },
      });
}

/** The route of a channel with one of its threads open beside it. */
export function threadLink(home: ChannelHome, channelId: string, threadId: string) {
  return home === null
    ? linkOptions({ to: "/dms/$channelId/threads/$threadId", params: { channelId, threadId } })
    : linkOptions({
        to: "/communities/$communityId/channels/$channelId/threads/$threadId",
        params: { communityId: home, channelId, threadId },
      });
}
