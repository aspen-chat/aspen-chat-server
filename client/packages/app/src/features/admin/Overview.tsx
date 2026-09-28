import type { AdminOverview } from "@aspen/protocol";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { headline } from "@/features/admin/format";
import type { AdminRead } from "@/features/admin/useAdminRead";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The deployment's totals, as a row of figures. */
export function Overview({ read }: { read: AdminRead<AdminOverview> }) {
  const m = useMessages();
  const { data, error, reload } = read;
  return (
    <Section id="admin-overview" title={m.admin.overview}>
      {error !== null && <ReadFailed error={error} onRetry={reload} />}
      <dl className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Figure
          label={m.admin.users}
          value={data === undefined ? "—" : headline(data.users)}
          detail={
            data === undefined
              ? undefined
              : format(m.admin.newThisWeek, { count: headline(data.newUsersThisWeek) })
          }
        />
        <Figure
          label={m.admin.communities}
          value={data === undefined ? "—" : headline(data.communities)}
        />
        <Figure
          label={m.admin.registration}
          value={
            data === undefined
              ? "—"
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
  value,
  detail,
  small = false,
}: {
  label: string;
  value: string;
  detail?: string | undefined;
  /** For a value that is words rather than a number. */
  small?: boolean;
}) {
  return (
    <div className="flex flex-col gap-1 rounded-lg border border-line bg-surface-raised p-4">
      <dt className="text-sm text-ink-muted">{label}</dt>
      <dd className={small ? "text-xl font-semibold" : "text-3xl font-semibold"}>{value}</dd>
      {detail !== undefined && <dd className="text-sm text-ink-muted">{detail}</dd>}
    </div>
  );
}
