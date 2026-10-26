import type { AspenSync, Message } from "@aspen/protocol";
import { decodeTags } from "@/features/mentions/tags";
import { growthText } from "@/features/messages/growth";
import { outcomeText } from "@/features/messages/poll";
import { displayNameOf } from "@/features/users/profile";
import { format, type Messages } from "@/i18n/messages";

/**
 * What a notification says of a message: who wrote it (by their nickname, in a community), that
 * and where as its title, and its text with tags as names, or for a message with no text of its
 * own what it records.
 */
export function describe(
  m: Messages,
  sync: AspenSync,
  message: Message,
): { name: string; title: string; body: string } {
  const store = sync.store;
  const author = store.user(message.author);
  const channel = store.channel(message.channelId);
  const place = channel?.parentChannel != null ? store.channel(channel.parentChannel) : channel;
  const nickname =
    place?.community == null ? undefined : store.nickname(place.community, message.author);
  const name = author === undefined ? m.unknownUser : (nickname ?? displayNameOf(author));
  const community = place?.community == null ? undefined : store.community(place.community);
  const title =
    place === undefined || community === undefined
      ? name
      : format(m.notifications.titleInChannel, {
          name,
          channel: place.name,
          community: community.name,
        });
  const roles = place?.community == null ? [] : store.roles(place.community);
  const { text } = decodeTags(
    message.content,
    (id) => store.user(id)?.name,
    (id) => roles.find((role) => role.id === id)?.name,
  );
  const body = text.trim() !== "" ? text : recorded(m, sync, message, name, community?.name);
  return { name, title, body };
}

/** What a message with no text of its own says, by its kind. */
function recorded(
  m: Messages,
  sync: AspenSync,
  message: Message,
  name: string,
  community: string | undefined,
): string {
  switch (message.kind) {
    case "poll":
      return m.notifications.poll;
    case "pollClosed": {
      const poll = message.poll == null ? undefined : sync.store.poll(message.poll);
      return poll === undefined
        ? m.notifications.pollClosed
        : `${format(m.poll.closedNotice, { question: poll.question })} ${outcomeText(m, poll)}`;
    }
    case "memberJoined":
      return format(m.memberJoined, { name });
    case "communityGrew":
      return growthText(m, message.growth, community ?? m.communityGrew.thisCommunity);
    default:
      return message.attachments.length > 0
        ? m.notifications.attachment
        : m.notifications.newMessage;
  }
}
