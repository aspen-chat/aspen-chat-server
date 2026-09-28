import { ApiProblemError, type ModerationEntry } from "@aspen/protocol";
import { useCallback, useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useSync, useUser } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { Cell, Table } from "@/features/admin/FleetHealth";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import type { Messages } from "@/i18n/messages";

const timeFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

/** A page of the log is what the server gives by default. */
const PAGE = 50;

/**
 * The moderation log, newest first, a page at a time: each use of Moderate any community that a
 * community's own permissions would not have allowed, and every reading of a DM by someone not
 * in it. It is how the deployment's administrators oversee its moderators.
 */
export function ModerationLog() {
  const m = useMessages();
  const sync = useSync();
  const [entries, setEntries] = useState<readonly ModerationEntry[]>([]);
  const [complete, setComplete] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(
    (before: string | undefined, isCurrent: () => boolean) => {
      sync.moderationLog(before).then(
        (page) => {
          if (!isCurrent()) {
            return;
          }
          setEntries((held) => (before === undefined ? page : [...held, ...page]));
          setComplete(page.length < PAGE);
          setError(null);
          setLoading(false);
        },
        (e: unknown) => {
          if (isCurrent()) {
            setError(e instanceof ApiProblemError ? e.message : String(e));
            setLoading(false);
          }
        },
      );
    },
    [sync],
  );

  useEffect(() => {
    let current = true;
    read(undefined, () => current);
    return () => {
      current = false;
    };
  }, [read]);

  return (
    <Section id="admin-moderation" title={m.admin.moderationLog} hint={m.admin.moderationLogHint}>
      {error !== null && (
        <ReadFailed
          error={error}
          onRetry={() => {
            setLoading(true);
            read(undefined, () => true);
          }}
        />
      )}
      {!loading && entries.length === 0 && error === null ? (
        <p className="text-sm text-ink-muted">{m.admin.noModeration}</p>
      ) : (
        <Table
          label={m.admin.moderationLog}
          headings={[
            { content: m.admin.logWhen },
            { content: m.admin.logWho },
            { content: m.admin.logWhat },
            { content: m.admin.logWhere },
          ]}
          numeric={[]}
          dimmed={loading}
        >
          {entries.map((entry) => (
            <tr key={entry.id}>
              <Cell>{timeFormat.format(new Date(entry.at))}</Cell>
              <Cell>
                <Actor userId={entry.actor ?? undefined} />
              </Cell>
              <Cell>{actionName(m, entry.action)}</Cell>
              <Cell>
                <code className="font-mono text-xs break-all">
                  {[entry.community, entry.channel, entry.subject]
                    .filter((part) => part != null)
                    .join(" · ")}
                </code>
              </Cell>
            </tr>
          ))}
        </Table>
      )}
      {!complete && entries.length > 0 && (
        <Button
          isDisabled={loading}
          onPress={() => {
            setLoading(true);
            read(entries.at(-1)?.id, () => true);
          }}
          className={secondaryButtonClass + " self-start"}
        >
          {m.admin.showMore}
        </Button>
      )}
    </Section>
  );
}

function actionName(m: Messages, action: string): string {
  const names: Record<string, string> = m.admin.moderationActions;
  return names[action] ?? action;
}

function Actor({ userId }: { userId: string | undefined }) {
  const m = useMessages();
  const user = useUser(userId);
  return <>{user === undefined ? m.unknownUser : displayNameOf(user)}</>;
}
