import type { AdminOverview } from "@aspen/protocol";
import { Link } from "@tanstack/react-router";
import type { AdminTab } from "@/features/admin/adminTabs";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { useFigures } from "@/features/admin/format";
import type { AdminRead } from "@/features/admin/useAdminRead";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";

/** The deployment's totals, as a row of figures. */
export function Overview({ read }: { read: AdminRead<AdminOverview> }) {
  const m = useMessages();
  const { headline } = useFigures();
  const { data, error, reload } = read;
  return (
    <Section id="admin-overview" title={m.admin.overview}>
      {error !== null && <ReadFailed error={error} onRetry={reload} />}
      <dl className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Figure
          label={m.admin.users}
          tab="users"
          value={data === undefined ? (error === null ? undefined : "—") : headline(data.users)}
          detail={
            data === undefined
              ? undefined
              : format(m.admin.newThisWeek, { count: headline(data.newUsersThisWeek) })
          }
        />
        <Figure
          label={m.admin.communities}
          tab="communities"
          value={
            data === undefined ? (error === null ? undefined : "—") : headline(data.communities)
          }
        />
        <Figure
          label={m.admin.registration}
          value={
            data === undefined
              ? error === null
                ? undefined
                : "—"
              : data.registrationInviteRequired
                ? m.admin.inviteOnly
                : m.admin.openRegistration
          }
          detail={m.admin.registrationHint}
          small
        />
      </dl>
    </Section>
  );
}

/** A headline figure: its label, the value, and a line of detail under it. */
function Figure({
  label,
  tab,
  value,
  detail,
  small = false,
}: {
  label: string;
  /**
   * The tab listing what it counts. Its label is then a link whose target covers the whole
   * panel, since a `<dl>` may hold no link around its groups.
   */
  tab?: AdminTab;
  /** `undefined` while it is on its way, a skeleton its size meanwhile. */
  value: string | undefined;
  detail?: string | undefined;
  /** For a value that is words rather than a number. */
  small?: boolean;
}) {
  return (
    <div
      className={`flex flex-col gap-1 rounded-lg border border-line bg-surface-raised p-4 ${tab === undefined ? "" : "relative has-[a:focus-visible]:ring-2 has-[a:focus-visible]:ring-accent/50 has-[a:hover]:bg-surface-hover"}`}
    >
      <dt className="text-sm text-ink-muted">
        {tab === undefined ? (
          label
        ) : (
          <Link
            to="/admin/$tab"
            params={{ tab }}
            className="outline-none after:absolute after:inset-0 after:rounded-lg hover:text-ink"
          >
            {label}
          </Link>
        )}
      </dt>
      <dd className={small ? "text-xl font-semibold" : "text-3xl font-semibold"}>
        {value ?? (
          <>
            <LoadingLabel />
            <Skeleton inline className={small ? "w-28" : "w-16"} />
          </>
        )}
      </dd>
      {detail !== undefined && <dd className="text-sm text-ink-muted">{detail}</dd>}
    </div>
  );
}
