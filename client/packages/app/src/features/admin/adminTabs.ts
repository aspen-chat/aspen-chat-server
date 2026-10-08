import { useMessages } from "@/i18n/context";
import type { Messages } from "@/i18n/messages";

/** The dashboard's tabs, in the rail's order. */
export const ADMIN_TABS = [
  "overview",
  "fleet",
  "invites",
  "roles",
  "reports",
  "reportCategories",
  "users",
  "communities",
  "settings",
  "newsletter",
  "federation",
  "plugins",
  "moderation",
  "transfers",
  "jobs",
] as const;
export type AdminTab = (typeof ADMIN_TABS)[number];

/** Whether `tab` names one of the dashboard's tabs, as a link to `/admin/{tab}` may not. */
export function isAdminTab(tab: string): tab is AdminTab {
  return (ADMIN_TABS as readonly string[]).includes(tab);
}

/** What the dashboard calls each tab, in its rail and wherever a link names one. */
export function adminTabLabel(m: Messages, tab: AdminTab): string {
  return {
    overview: m.admin.overview,
    fleet: m.admin.fleet,
    invites: m.admin.invites,
    roles: m.admin.deploymentRoles,
    reports: m.reports.title,
    reportCategories: m.reports.categoriesTitle,
    users: m.admin.users,
    communities: m.admin.communities,
    settings: m.admin.settings,
    newsletter: m.email.newsletterTitle,
    federation: m.federation.title,
    plugins: m.plugins.adminTab,
    moderation: m.admin.moderationLog,
    transfers: m.admin.fileTransfers,
    jobs: m.admin.jobs,
  }[tab];
}

/** `adminTabLabel` in the language shown. */
export function useAdminTabLabel(): (tab: AdminTab) => string {
  const m = useMessages();
  return (tab) => adminTabLabel(m, tab);
}
