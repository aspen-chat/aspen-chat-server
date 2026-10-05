import type { AspenSync, Message } from "@aspen/protocol";
import { decodeTags } from "@/features/mentions/tags";
import { displayNameOf } from "@/features/users/profile";
import { format, type Messages } from "@/i18n/messages";

/**
 * What a notification says of a message: who wrote it (by their nickname, in a community), that
 * and where as its title, and its text with tags as names.
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
  const body =
    text.trim() !== ""
      ? text
      : message.kind === "poll"
        ? m.notifications.poll
        : message.attachments.length > 0
          ? m.notifications.attachment
          : m.notifications.newMessage;
  return { name, title, body };
}
