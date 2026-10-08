import {
  dm,
  dmMessages,
  dmRecord,
  freshId,
  me,
  general,
  generalMessages,
  generalPage,
  lunchPoll,
  messageId,
  minutesAgo,
  reactedId,
  reactionSummaries,
  searchMessages,
  thread,
  threadRecord,
  threadReplies,
  users,
} from "../fixtures";
import { type Asked, type WorldRoute, reply } from "../reply";

/** The routes of message search and history, the thread, Bob's poll, and the caller's preferences and presence. */
export function messageRoutes({
  request,
  url,
  path,
  poll,
  publish,
  personal,
}: Asked): WorldRoute[] {
  const { saves, follows } = personal;
  const everyMessage = [...generalMessages, ...threadReplies, ...dmMessages];
  return [
    // Saving and following answer as the server does, and tell the caller's devices by event.
    [
      "GET",
      /^\/users\/@me\/saved-messages$/,
      () =>
        Array.from(saves, ([message, id]) => ({ id, message })).sort((a, b) =>
          b.id.localeCompare(a.id),
        ),
    ],
    [
      "GET",
      /^\/users\/@me\/saved-messages\/messages$/,
      () => ({
        data: Array.from(saves, ([message, id]) => ({ id, message }))
          .sort((a, b) => b.id.localeCompare(a.id))
          .flatMap(({ message }) => everyMessage.filter((m) => m.id === message)),
        included: { users, channels: [threadRecord] },
      }),
    ],
    [
      "PUT",
      /^\/users\/@me\/saved-messages\/[^/]+$/,
      () => {
        const message = path.split("/").pop() ?? "";
        const id = saves.get(message) ?? freshId();
        const made = !saves.has(message);
        saves.set(message, id);
        publish({ serverEvent: "savedMessageChanged", message, saved: id });
        return reply({ id, message }, made ? 201 : 200);
      },
    ],
    [
      "DELETE",
      /^\/users\/@me\/saved-messages\/[^/]+$/,
      () => {
        const message = path.split("/").pop() ?? "";
        if (saves.delete(message)) {
          publish({ serverEvent: "savedMessageChanged", message, saved: null });
        }
        return reply(null, 204);
      },
    ],
    [
      "GET",
      /^\/users\/@me\/thread-follows$/,
      () => Array.from(follows, (followed) => ({ thread: followed, followedAt: minutesAgo(1) })),
    ],
    [
      "PUT",
      /^\/channels\/[^/]+\/follows\/@me$/,
      () => {
        const followed = path.split("/")[2] ?? "";
        follows.add(followed);
        publish({ serverEvent: "threadFollowChanged", thread: followed, following: true });
        return reply({ thread: followed, followedAt: minutesAgo(0) }, 201);
      },
    ],
    [
      "DELETE",
      /^\/channels\/[^/]+\/follows\/@me$/,
      () => {
        const followed = path.split("/")[2] ?? "";
        follows.delete(followed);
        publish({ serverEvent: "threadFollowChanged", thread: followed, following: false });
        return reply(null, 204);
      },
    ],
    // The feed holds Bob's DMs, and his replies in the thread once the caller follows it.
    [
      "GET",
      /^\/users\/@me\/activity$/,
      () => ({
        data: [
          ...(url.searchParams.get("filter[dms]") === "false" ? [] : dmMessages),
          ...(follows.has(thread) ? threadReplies : []),
        ]
          .filter((m) => m.author !== me)
          .sort((a, b) => b.id.localeCompare(a.id)),
        included: { users, channels: [threadRecord, dmRecord] },
      }),
    ],
    ["GET", /^\/messages$/, () => searchMessages(url)],
    ["GET", /^\/users\/@me\/preferences$/, () => ({ values: {}, updatedAt: minutesAgo(600) })],
    [
      "PATCH",
      /^\/users\/@me\/preferences$/,
      () => ({ values: request.postDataJSON() as unknown, updatedAt: minutesAgo(0) }),
    ],
    ["GET", /^\/users\/statuses$/, () => users.map((u) => ({ id: u.id, onlineStatus: "online" }))],
    [
      "GET",
      new RegExp(`^/channels/${general}/messages$`),
      () => ({
        data: generalPage(url),
        included: {
          users,
          channels: [threadRecord],
          messages: [threadReplies[1]],
          reactions: generalPage(url).some((m) => m.id === reactedId) ? reactionSummaries : [],
        },
      }),
    ],
    [
      "GET",
      new RegExp(`^/channels/${thread}/messages$`),
      () => ({ data: [...threadReplies].reverse(), included: { users } }),
    ],
    [
      "GET",
      new RegExp(`^/channels/${dm}/messages$`),
      () => ({ data: dmMessages, included: { users } }),
    ],
    ["GET", /^\/channels\/[^/]+\/messages$/, () => ({ data: [], included: { users } })],
    ["GET", new RegExp(`^/channels/${thread}$`), () => threadRecord],
    ["GET", new RegExp(`^/polls/${lunchPoll}$`), poll.read],
    [
      "POST",
      new RegExp(`^/polls/${lunchPoll}/write-ins$`),
      () => poll.writeIn((request.postDataJSON() as { label: string }).label),
    ],
    [
      "DELETE",
      new RegExp(`^/polls/${lunchPoll}/write-ins/\\d+$`),
      () => poll.remove(Number(path.split("/").pop())),
    ],
    [
      "GET",
      new RegExp(`^/messages/${messageId(203)}$`),
      () => ({
        data: generalMessages.find((m) => m.id === messageId(203)),
        included: { users, channels: [threadRecord], attachments: [], polls: [], pollVotes: [] },
      }),
    ],
  ];
}
