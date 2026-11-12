import type { AspenSync, Message } from "@aspen/protocol";
import { decodeCustomEmoji } from "@/features/emoji/customEmoji";
import { decodeTags } from "@/features/mentions/tags";
import { growthText } from "@/features/messages/growth";
import { isImageType } from "@/features/messages/images";
import { maskSpoilers } from "@/features/messages/maskSpoilers";
import { outcomeText } from "@/features/messages/poll";
import { displayNameOf } from "@/features/users/profile";
import { format, type Messages } from "@/i18n/messages";

/**
 * What a notification says of a message, from what the store holds: who wrote it (by their
 * nickname, in a community), that and where as its title, and as its body its text with tags
 * and its community's emoji as names and each spoiler as a word for one (`maskSpoilers`), or
 * for a message with no text of its own what it records. A system notification shows the
 * deployment's own wording of the body instead where it can read it (`messageBody`).
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
  // Spoilers first: a name may hold what would read as a marker.
  const text = decodeCustomEmoji(
    decodeTags(
      maskSpoilers(message.content, m.replies.spoiler),
      (id) => store.user(id)?.name,
      (id) => roles.find((role) => role.id === id)?.name,
    ),
    place?.community == null ? [] : store.customEmoji(place.community),
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
      return held(
        m,
        message.attachments.map((id) => sync.store.attachment(id)?.mimeType),
      );
  }
}

/**
 * What a message says by its files alone, each given as the type it was uploaded as, or
 * `undefined` for one not held: pictures when every one is known to be, as the server's
 * notification text says of the same message (`app::message::notification`).
 */
export function held(m: Messages, types: readonly (string | undefined)[]): string {
  if (types.length === 0) {
    return m.notifications.newMessage;
  }
  return types.every((type) => type !== undefined && isImageType(type))
    ? format(m.notifications.pictures, { count: types.length })
    : m.notifications.attachment;
}
