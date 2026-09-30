import type { Permission } from "@aspen/protocol";

/**
 * Every permission, grouped as the role editor shows them: what someone may do to the
 * community, to its members, and as a moderator, and then in text and voice channels, which
 * are also the ones a channel's access settings adjust.
 */
export const PERMISSION_GROUPS = [
  { key: "community", permissions: ["manageCommunity", "manageChannels", "manageCategories"] },
  {
    key: "members",
    permissions: [
      "createInvites",
      "manageInvites",
      "manageRoles",
      "assignRoles",
      "removeMembers",
      "addBots",
    ],
  },
  { key: "moderation", permissions: ["manageMessages", "pinMessages", "manageCalls"] },
  {
    key: "text",
    permissions: [
      "viewChannel",
      "sendMessages",
      "attachFiles",
      "addReactions",
      "startThreads",
      "sendInThreads",
      "createPolls",
      "mentionMembers",
      "mentionRoles",
      "mentionEveryone",
    ],
  },
  {
    key: "voice",
    permissions: ["joinVoice", "speak", "shareScreen", "useCamera", "transferFiles"],
  },
] as const satisfies readonly { key: string; permissions: readonly Permission[] }[];

/** The groups a channel or category override can adjust. */
export const CHANNEL_GROUPS = PERMISSION_GROUPS.filter(
  (g) => g.key === "text" || g.key === "voice",
);
