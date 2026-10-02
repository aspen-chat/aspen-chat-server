import {
  dm,
  dmMessages,
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
import type { Asked, WorldRoute } from "../reply";

/** The routes of message search and history, the thread, Bob's poll, and the caller's preferences and presence. */
export function messageRoutes({ request, url, path, poll }: Asked): WorldRoute[] {
  return [
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
