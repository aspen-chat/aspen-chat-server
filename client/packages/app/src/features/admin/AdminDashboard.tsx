import {
  ChartLineUpIcon,
  FileArrowUpIcon,
  FlagIcon,
  GearSixIcon,
  GlobeIcon,
  ListBulletsIcon,
  HardDrivesIcon,
  IdentificationBadgeIcon,
  ScrollIcon,
  TicketIcon,
  UsersIcon,
  UsersThreeIcon,
  type Icon,
} from "@phosphor-icons/react";
import { Link, Navigate } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Button } from "react-aria-components";
import { useCallback } from "react";
import { planeClass } from "@/features/invites/dialog";
import { useDeploymentPermissions, useIsAdmin, useOpenReports, useSync } from "@/api/hooks";
import { ReportCategoriesSection } from "@/features/admin/ReportCategories";
import { ReportsSection } from "@/features/admin/Reports";
import { MentionBadge } from "@/features/mentions/MentionBadge";
import { DeploymentProfileSection } from "@/features/admin/DeploymentProfile";
import { DeploymentSettingsSection } from "@/features/admin/DeploymentSettings";
import { DeploymentRolesSection } from "@/features/admin/DeploymentRoles";
import { FederationSection } from "@/features/admin/Federation";
import { useDeploymentRoles } from "@/features/admin/deploymentRoleRecords";
import { FileTransferLog } from "@/features/admin/FileTransferLog";
import { ModerationLog } from "@/features/admin/ModerationLog";
import { useAdminRead } from "@/features/admin/useAdminRead";
import { SidebarFooter } from "@/features/layout/SidebarFooter";
import { linkButtonClass } from "@/features/auth/styles";
import { CommunityDirectory, UserDirectory } from "@/features/admin/Directories";
import { FleetHealth } from "@/features/admin/FleetHealth";
import { Growth } from "@/features/admin/Growth";
import { Overview } from "@/features/admin/Overview";
import { RegistrationInvites } from "@/features/admin/RegistrationInvites";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The dashboard's tabs, in the rail's order. */
const ADMIN_TABS = [
  "overview",
  "fleet",
  "invites",
  "roles",
  "reports",
  "reportCategories",
  "users",
  "communities",
  "settings",
  "federation",
  "moderation",
  "transfers",
] as const;
type AdminTab = (typeof ADMIN_TABS)[number];

/** Which tabs the caller may open: each needs the deployment permission its content does. */
function useAllowedTabs(): AdminTab[] {
  const permissions = useDeploymentPermissions();
  const view = permissions.has("viewDashboard");
  const directories =
    view ||
    permissions.has("moderateCommunities") ||
    permissions.has("banUsers") ||
    permissions.has("reviewReports");
  const allowed: Record<AdminTab, boolean> = {
    overview: view,
    fleet: view,
    invites: permissions.has("manageRegistrationInvites"),
    roles: true,
    reports: permissions.has("reviewReports"),
    reportCategories: permissions.has("manageReportCategories"),
    users: directories,
    communities: directories,
    settings: permissions.has("manageDeploymentSettings"),
    federation: permissions.has("manageFederation"),
    moderation: view,
    transfers: view,
  };
  return ADMIN_TABS.filter((tab) => allowed[tab]);
}

function useTabLabel(): (tab: AdminTab) => string {
  const m = useMessages();
  return (tab) =>
    ({
      overview: m.admin.overview,
      fleet: m.admin.fleet,
      invites: m.admin.invites,
      roles: m.admin.deploymentRoles,
      reports: m.reports.title,
      reportCategories: m.reports.categoriesTitle,
      users: m.admin.users,
      communities: m.admin.communities,
      settings: m.admin.settings,
      federation: m.federation.title,
      moderation: m.admin.moderationLog,
      transfers: m.admin.fileTransfers,
    })[tab];
}

const TAB_ICONS: Record<AdminTab, Icon> = {
  overview: ChartLineUpIcon,
  fleet: HardDrivesIcon,
  invites: TicketIcon,
  roles: IdentificationBadgeIcon,
  reports: FlagIcon,
  reportCategories: ListBulletsIcon,
  users: UsersIcon,
  communities: UsersThreeIcon,
  settings: GearSixIcon,
  federation: GlobeIcon,
  moderation: ScrollIcon,
  transfers: FileArrowUpIcon,
};

/**
 * The Administration Dashboard, `/admin/{tab}`: a rail of tabs beside the one open, which a
 * one-pane screen sets across the top instead. The user bar (`SidebarFooter`) is at the foot of
 * the rail, or of the screen on a one-pane screen. The tabs are the deployment's totals and their
 * growth, the health of its servers, registration invites, its roles, the reports people made
 * and the categories they make them in, searchable lists of its users and communities, the name and icon it welcomes people with, federation with other
 * deployments, the moderation log, and the record
 * of file transfers. Each shows only to those with the deployment permission it needs; the
 * server refuses everyone else whatever this page shows. `/admin`, or a tab the caller may not
 * open, goes to the first they may.
 */
export function AdminDashboard({ tab }: { tab: string | undefined }) {
  const m = useMessages();
  const admin = useIsAdmin();
  const allowed = useAllowedTabs();
  const label = useTabLabel();
  const openReports = useOpenReports() ?? 0;
  const open = allowed.find((t) => t === tab);
  const first = allowed[0];
  if (admin && open === undefined && first !== undefined) {
    return <Navigate to="/admin/$tab" params={{ tab: first }} replace />;
  }
  return (
    // A grid where the rail stands beside the tab, so the user bar can sit at the rail's foot
    // while staying last in reading order, as it is on a one-pane screen.
    <main className="flex min-w-0 flex-1 flex-col bg-surface md:grid md:grid-cols-[14rem_minmax(0,1fr)] md:grid-rows-[minmax(0,1fr)_auto]">
      <nav
        aria-label={m.admin.tabs}
        className="flex shrink-0 flex-col gap-2 border-b border-line bg-surface-sunken p-3 md:col-start-1 md:row-start-1 md:overflow-y-auto md:border-e md:border-b-0"
      >
        <h1 className="px-2 text-lg font-semibold">{m.admin.title}</h1>
        {admin && (
          <ul className="-mx-1 flex gap-1 overflow-x-auto px-1 md:flex-col md:overflow-visible">
            {allowed.map((t) => {
              const TabIcon = TAB_ICONS[t];
              return (
                <li key={t} className="shrink-0">
                  <Link
                    to="/admin/$tab"
                    params={{ tab: t }}
                    aria-current={t === open ? "page" : undefined}
                    ref={t === open ? showOpenTab : undefined}
                    className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm whitespace-nowrap text-ink-muted outline-none hover:bg-surface-hover hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50 aria-[current=page]:bg-surface-raised aria-[current=page]:font-medium aria-[current=page]:text-accent aria-[current=page]:shadow-sm"
                  >
                    <TabIcon size={18} aria-hidden="true" />
                    {label(t)}
                    {t === "reports" && (
                      <>
                        <MentionBadge
                          count={openReports}
                          title={format(m.admin.reportsWaiting, { count: String(openReports) })}
                          className="ms-auto"
                        />
                        {openReports > 0 && (
                          <span className="sr-only">
                            {format(m.admin.reportsWaiting, { count: String(openReports) })}
                          </span>
                        )}
                      </>
                    )}
                  </Link>
                </li>
              );
            })}
          </ul>
        )}
      </nav>
      <div className="min-h-0 min-w-0 flex-1 overflow-y-auto md:col-start-2 md:row-span-2 md:row-start-1">
        <div className="mx-auto flex max-w-5xl flex-col gap-4 px-4 py-6 md:px-6">
          {!admin ? (
            <p className="text-ink-muted">{m.admin.notAllowed}</p>
          ) : open === undefined ? null : (
            <TabContent tab={open} />
          )}
        </div>
      </div>
      <div className="shrink-0 bg-surface-sunken md:col-start-1 md:row-start-2 md:border-e md:border-line">
        <SidebarFooter />
      </div>
    </main>
  );
}

/** Keeps the open tab in view where the tabs scroll, as across the top of a phone. */
function showOpenTab(link: HTMLAnchorElement | null) {
  link?.scrollIntoView({ block: "nearest", inline: "nearest" });
}

function TabContent({ tab }: { tab: AdminTab }) {
  const permissions = useDeploymentPermissions();
  const roles = useDeploymentRoles();
  switch (tab) {
    case "overview":
      return <Totals />;
    case "fleet":
      return <FleetHealth />;
    case "invites":
      return <Invites view={permissions.has("viewDashboard")} />;
    case "roles":
      return <DeploymentRolesSection read={roles} />;
    case "reports":
      return <ReportsSection />;
    case "reportCategories":
      return <ReportCategoriesSection />;
    case "users":
      return <UserDirectory roles={roles.data} />;
    case "communities":
      return <CommunityDirectory />;
    case "settings":
      return (
        <>
          <DeploymentProfileSection />
          <DeploymentSettingsSection />
        </>
      );
    case "federation":
      return <FederationSection />;
    case "moderation":
      return <ModerationLog />;
    case "transfers":
      return <FileTransferLog />;
  }
}

/** The totals and their growth, which share one read. */
function Totals() {
  const overview = useOverview();
  return (
    <>
      <Overview read={overview} />
      <Growth />
    </>
  );
}

/** Registration invites, whose hint says whether the server requires them when that is known. */
function Invites({ view }: { view: boolean }) {
  return view ? <InvitesWithTotals /> : <RegistrationInvites inviteRequired={undefined} />;
}

function InvitesWithTotals() {
  const overview = useOverview();
  return <RegistrationInvites inviteRequired={overview.data?.registrationInviteRequired} />;
}

function useOverview() {
  const sync = useSync();
  const load = useCallback(() => sync.admin.adminOverview(), [sync]);
  return useAdminRead(load);
}

/** A section of the dashboard, drawn as a plane: a heading, an optional line under it, and its content. */
export function Section({
  id,
  title,
  hint,
  children,
}: {
  id: string;
  title: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={id} className={planeClass}>
      <div>
        <h2 id={id} className="text-lg font-semibold">
          {title}
        </h2>
        {hint !== undefined && <p className="text-sm text-ink-muted">{hint}</p>}
      </div>
      {children}
    </section>
  );
}

/** A read that failed, with a way to try it again. */
export function ReadFailed({ error, onRetry }: { error: string; onRetry: () => void }) {
  const m = useMessages();
  return (
    <p role="alert" className="flex flex-wrap items-center gap-2 text-sm text-danger">
      {format(m.admin.loadFailed, { detail: error })}
      <Button onPress={onRetry} className={linkButtonClass}>
        {m.admin.retry}
      </Button>
    </p>
  );
}
