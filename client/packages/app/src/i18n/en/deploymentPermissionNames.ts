export const deploymentPermissionNames = {
  viewDashboard: {
    name: "View the dashboard",
    hint: "See the server's totals, growth, health, users, communities, and the moderation log.",
  },
  manageRegistrationInvites: {
    name: "Manage registration invites",
    hint: "Make and revoke the invites new accounts are created with.",
  },
  manageVoiceServers: {
    name: "Manage voice servers",
    hint: "Add, change, and remove the servers calls run on.",
  },
  manageDeploymentRoles: {
    name: "Manage deployment roles",
    hint: "Create, change, reorder, give, and take deployment roles below their own highest.",
  },
  manageFederation: {
    name: "Manage federation",
    hint: "Open and close the gates to other deployments, add and forget them, check and accept their keys, and edit the lists that decide who may come and go.",
  },
  moderateCommunities: {
    name: "Moderate any community",
    hint: "See everything on the server, DMs included, and the record of files sent in calls; delete messages, attachments, reactions, members, channels, and communities. Includes Remove content. Every use is logged.",
  },
  reviewReports: {
    name: "Review reports",
    hint: "Read what people report, with the messages around a reported one (a DM's is logged), dismiss it, or resolve it with a warning from the server's moderators.",
  },
  removeContent: {
    name: "Remove content",
    hint: "Delete a reported message, clear a reported nickname, reset a reported profile, and delete a banned person's recent messages. Every use is logged.",
  },
  manageReportCategories: {
    name: "Manage report categories",
    hint: "Add, rename, reorder, and hide the categories people choose from when they report something.",
  },
  banUsers: {
    name: "Ban users",
    hint: "Ban accounts from the whole server, ending their sign-ins, and lift bans. Every use is logged.",
  },
  messageAnyUser: {
    name: "Message any user",
    hint: "Start a DM with anyone, whatever communities they share and whoever blocked whom.",
  },
  manageDeploymentSettings: {
    name: "Manage deployment settings",
    hint: "Change the server's name and icon, and its policies: registration invites, second factors, email, bots, community limits, and files in calls; delete bots whose owners have deleted their accounts.",
  },
  managePlugins: {
    name: "Manage plugins",
    hint: "Turn installed plugins on and off for the whole server, change their settings, and choose where they run and in what order. Installing them is done on the server itself.",
  },
  sendNewsletters: {
    name: "Send newsletters",
    hint: "Write posts for the server's email newsletter, send tests to yourself, and send them to every subscriber.",
  },
} as const;
