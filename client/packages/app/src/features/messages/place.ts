import type { Channel, Message } from "@aspen/protocol";
import { useChannelOnDemand, useCommunity } from "@/api/hooks";
import { messageLink, threadLink, type ChannelHome } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Where a message was said, for lists that show messages out of their channels (search, saved
 * messages, the activity feed): the channel that governs it (a thread's parent), the thread
 * when it is a reply in one, the home its links are built on, the route that goes to it (in
 * its thread, for a reply), and a line naming the place. Run in the message's deployment's
 * scope; the channels are read on demand when the store lacks them.
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
  const m = useMessages();
  const channel = useChannelOnDemand(message?.channelId ?? "");
  const parent = useChannelOnDemand(channel?.parentChannel ?? "");
  const thread = channel?.parentChannel != null ? channel : undefined;
  const place = thread === undefined ? channel : parent;
  const community = useCommunity(place?.community ?? "");
  const home: ChannelHome = { domain, community: place?.community ?? null };
  const link =
    message === undefined
      ? undefined
      : thread?.parentChannel != null
        ? threadLink(home, thread.parentChannel, thread.id)
        : messageLink(home, message.channelId, message.id);
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
  return { place, thread, home, link, where };
}
