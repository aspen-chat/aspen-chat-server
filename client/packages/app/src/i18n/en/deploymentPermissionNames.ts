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
  manageBots: {
    name: "Manage bots",
    hint: "Delete bots whose owners have deleted their accounts.",
  },
  manageFederation: {
    name: "Manage federation",
    hint: "Add and forget other deployments, check and accept their keys, and edit the lists that decide who may come and go.",
  },
  moderateCommunities: {
    name: "Moderate any community",
    hint: "See everything on the server, DMs included, and delete messages, attachments, reactions, members, channels, and communities. Every use is logged.",
  },
} as const;
