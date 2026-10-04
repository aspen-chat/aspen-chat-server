export const permissionNames = {
  manageCommunity: {
    name: "Manage community",
    hint: "Rename the community and change its icon.",
  },
  manageChannels: {
    name: "Manage channels",
    hint: "Create, edit, arrange, and delete channels, and set who can use them.",
  },
  manageCategories: {
    name: "Manage categories",
    hint: "Create, rename, and delete categories, and set who can use their channels.",
  },
  createInvites: { name: "Create invites", hint: "Make invite links for new members." },
  manageInvites: {
    name: "Manage invites",
    hint: "See, change, and revoke everyone's invites, not just their own.",
  },
  manageRoles: {
    name: "Manage roles",
    hint: "Create, edit, reorder, and delete roles below their highest one.",
  },
  assignRoles: {
    name: "Assign roles",
    hint: "Give members roles below their highest one, and take them away.",
  },
  removeMembers: {
    name: "Remove members",
    hint: "Remove members ranked below them. They can come back with an invite.",
  },
  banMembers: {
    name: "Ban members",
    hint: "Ban members below your highest role, with a reason and for a time, and lift bans.",
  },
  manageMessages: {
    name: "Manage messages",
    hint: "Delete anyone's messages and take down poll answers.",
  },
  pinMessages: { name: "Pin messages", hint: "Pin and unpin messages in channels." },
  manageCalls: {
    name: "Manage calls",
    hint: "Mute people in calls for everyone, and remove them from calls.",
  },
  addBots: {
    name: "Add bots",
    hint: "Add bots to the community from their links. Giving a bot permissions also takes Manage roles and Assign roles.",
  },
  manageCustomEmoji: {
    name: "Manage custom emoji",
    hint: "Add, rename, and remove the community's own emoji.",
  },
  viewChannel: { name: "View channels", hint: "See a channel and read its history." },
  sendMessages: { name: "Send messages", hint: "Post messages in channels." },
  attachFiles: { name: "Attach files", hint: "Add files and pictures to messages." },
  addReactions: { name: "Add reactions", hint: "React to messages with emoji." },
  startThreads: { name: "Start threads", hint: "Open a thread from a message." },
  sendInThreads: { name: "Send messages in threads", hint: "Reply in threads." },
  createPolls: { name: "Create polls", hint: "Post polls." },
  joinVoice: { name: "Join voice", hint: "Join calls in voice channels." },
  speak: { name: "Speak", hint: "Talk in calls. Without it, people join to listen." },
  shareScreen: { name: "Share screen", hint: "Share a screen, window, or game in calls." },
  transferFiles: {
    name: "Transfer files",
    hint: "Offer files to the others in a call, sent straight from device to device.",
  },
  useCamera: { name: "Use camera", hint: "Turn on a camera in calls." },
  mentionMembers: {
    name: "Mention members",
    hint: "Tag people in messages, which they're told of.",
  },
  mentionRoles: {
    name: "Mention roles",
    hint: "Tag a role, telling everyone who holds it.",
  },
  mentionEveryone: {
    name: "Mention everyone",
    hint: "Tag @everyone, telling everyone who can see the channel.",
  },
  managePlugins: {
    name: "Manage plugins",
    hint: "Turn this server's plugins on and off here, change their settings, and read them.",
  },
} as const;
