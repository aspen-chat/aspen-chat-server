import type { Channel, Message } from "@aspen/protocol";
import { useChannelOnDemand, useCommunity } from "@/api/hooks";
import { messageLink, threadLink, type ChannelHome } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Where a channel, DM, or thread is, for lists that show what was said out of its channel: the
 * channel that governs it (a thread's parent), the thread when it is one, the home its links
 * are built on, and a line naming the place. Run in its deployment's scope; the channels are
 * read on demand when the store lacks them.
 */
export function useChannelPlace(
  channelId: string,
  domain: string | null,
): {
  place: Channel | undefined;
  thread: Channel | undefined;
  home: ChannelHome;
  where: string;
} {
  const m = useMessages();
  const channel = useChannelOnDemand(channelId);
  const parent = useChannelOnDemand(channel?.parentChannel ?? "");
  const thread = channel?.parentChannel != null ? channel : undefined;
  const place = thread === undefined ? channel : parent;
  const community = useCommunity(place?.community ?? "");
  const home: ChannelHome = { domain, community: place?.community ?? null };
  const where =
    place === undefined
      ? ""
      : place.ty === "dm"
        ? m.search.inDm
        : place.ty === "groupDm"
          ? m.search.inGroupDm
          : thread !== undefined
            ? format(m.search.inThread, { channel: place.name, community: community?.name ?? "" })
            : format(m.search.inChannel, { channel: place.name, community: community?.name ?? "" });
  return { place, thread, home, where };
}

/**
 * Where a message was said (`useChannelPlace`, of its channel), for lists that show messages
 * out of their channels (search, saved messages, the activity feed), and the route that goes to
 * it (in its thread, for a reply).
 */
export function useMessagePlace(
  message: Message | undefined,
  domain: string | null,
): {
  place: Channel | undefined;
  thread: Channel | undefined;
  home: ChannelHome;
  link: ReturnType<typeof messageLink> | ReturnType<typeof threadLink> | undefined;
  where: string;
} {
  const { place, thread, home, where } = useChannelPlace(message?.channelId ?? "", domain);
  const link =
    message === undefined
      ? undefined
      : thread?.parentChannel != null
        ? threadLink(home, thread.parentChannel, thread.id)
        : messageLink(home, message.channelId, message.id);
  return { place, thread, home, link, where };
}
