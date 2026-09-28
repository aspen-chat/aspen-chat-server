import type { ReactNode } from "react";
import { Button } from "react-aria-components";
import { useCallback } from "react";
import { useDeploymentPermissions, useIsAdmin, useSync } from "@/api/hooks";
import { DeploymentRolesSection } from "@/features/admin/DeploymentRoles";
import { FederationSection } from "@/features/admin/Federation";
import { useDeploymentRoles } from "@/features/admin/deploymentRoles";
import { ModerationLog } from "@/features/admin/ModerationLog";
import { useAdminRead } from "@/features/admin/useAdminRead";
import { linkButtonClass } from "@/features/auth/styles";
import { CommunityDirectory, UserDirectory } from "@/features/admin/Directories";
import { FleetHealth } from "@/features/admin/FleetHealth";
import { Growth } from "@/features/admin/Growth";
import { Overview } from "@/features/admin/Overview";
import { RegistrationInvites } from "@/features/admin/RegistrationInvites";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The Administration Dashboard, `/admin`: the deployment's totals, the health of its servers,
 * registration invites, its roles, searchable lists of its users and communities, federation
 * with other deployments, and the moderation log. Each part shows only to those with the deployment permission it needs; the
 * server refuses everyone else whatever this page shows.
 */
export function AdminDashboard() {
  const m = useMessages();
  const admin = useIsAdmin();
  return (
    <main className="min-w-0 flex-1 overflow-y-auto bg-surface">
      <div className="mx-auto flex max-w-5xl flex-col gap-8 px-4 py-6 md:px-6">
        <h1 className="text-2xl font-semibold">{m.admin.title}</h1>
        {admin ? <Sections /> : <p className="text-ink-muted">{m.admin.notAllowed}</p>}
      </div>
    </main>
  );
}

/** The dashboard's sections the caller may see. */
function Sections() {
  const permissions = useDeploymentPermissions();
  const view = permissions.has("viewDashboard");
  const moderate = permissions.has("moderateCommunities");
  const roles = useDeploymentRoles();
  return (
    <>
      {view && <Totals />}
      {permissions.has("manageRegistrationInvites") && <Invites view={view} />}
      <DeploymentRolesSection read={roles} />
      {(view || moderate) && (
        <>
          <UserDirectory roles={roles.data} />
          <CommunityDirectory />
        </>
      )}
      {permissions.has("manageFederation") && <FederationSection />}
      {view && <ModerationLog />}
    </>
  );
}

/** The totals, their growth, and the servers' health, which share one read. */
function Totals() {
  const overview = useOverview();
  return (
    <>
      <Overview read={overview} />
      <Growth />
      <FleetHealth />
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
  const load = useCallback(() => sync.adminOverview(), [sync]);
  return useAdminRead(load);
}

/** A section of the dashboard: a heading, an optional line under it, and its content. */
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
    <section aria-labelledby={id} className="flex flex-col gap-3">
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
