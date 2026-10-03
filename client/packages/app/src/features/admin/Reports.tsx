import {
  ApiProblemError,
  type Channel,
  type Community,
  type ReportCase,
  type ReportCaseList,
  type ReportContext,
  type ReportStatus,
  type ReviewedMessage,
  type User,
} from "@aspen/protocol";
import { useCallback, useEffect, useState } from "react";
import { Button, ToggleButton, ToggleButtonGroup } from "react-aria-components";
import { useReportsChanges, useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { ResolveDialog } from "@/features/admin/ResolveDialog";
import { useAdminRead } from "@/features/admin/useAdminRead";
import { Avatar } from "@/features/communities/Avatar";
import { secondaryButtonClass, toggleChipClass } from "@/features/invites/dialog";
import { Skeleton } from "@/features/layout/Skeleton";
import { toast } from "@/features/layout/toast";
import { EmbeddedMessage } from "@/features/messages/EmbeddedMessage";
import { BotBadge } from "@/features/users/BotBadge";
import { ProfileSnapshotCard } from "@/features/users/ProfileSnapshotCard";
import { useAspectList } from "@/features/users/profileAspects";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };
/** How many cases a page lists, and the most the server lists at once. */
const PAGE = 15;
const MAX_LISTED = 100;
const STATUSES: readonly ReportStatus[] = ["open", "resolved", "dismissed"];

/** What a page of cases names, by id, for drawing each case. */
interface Named {
  users: ReadonlyMap<string, User>;
  messages: ReadonlyMap<string, ReviewedMessage>;
  channels: ReadonlyMap<string, Channel>;
  communities: ReadonlyMap<string, Community>;
  categories: ReadonlyMap<string, string>;
}

function namedBy(list: ReportCaseList): Named {
  return {
    users: new Map(list.users.map((u) => [u.id, u])),
    messages: new Map(list.messages.map((m) => [m.message.id, m])),
    channels: new Map(list.channels.map((c) => [c.id, c])),
    communities: new Map(list.communities.map((c) => [c.id, c])),
    categories: new Map(list.categories.map((c) => [c.id, c.name])),
  };
}

/**
 * The reports people made, for holders of Review reports: open ones to act on or dismiss,
 * resolved ones with what was done, and dismissed ones to restore. Each case shows what was
 * reported (the message, deleted or not, with a way to read the conversation around it, or the
 * profile as reported beside how it is now) and every report of it. The list reads itself again
 * whenever what awaits review changes (`reportsChanged`).
 */
export function ReportsSection() {
  const m = useMessages();
  const sync = useSync();
  const changes = useReportsChanges();
  const [status, setStatus] = useState<ReportStatus>("open");
  const [limit, setLimit] = useState(PAGE);
  const load = useCallback(
    () =>
      Promise.all([sync.admin.reportCases(status, { limit }), sync.admin.reportCounts()]).then(
        ([list, counts]) => {
          // The people and files they name are drawn from the cache, like everyone else's.
          sync.store.ingest({ users: list.users, attachments: list.attachments });
          return { list, counts };
        },
      ),
    [sync, status, limit],
  );
  const read = useAdminRead(load);
  // A change to what awaits review reads the page again.
  const [heard, setHeard] = useState(changes);
  if (heard !== changes) {
    setHeard(changes);
    read.reload();
  }
  const list = read.data?.list;
  const open = read.data?.counts.open;
  const label = (s: ReportStatus) =>
    s === "open"
      ? open === undefined
        ? m.reports.open
        : format(m.reports.openCount, { count: String(open) })
      : s === "resolved"
        ? m.reports.resolved
        : m.reports.dismissed;
  const none =
    status === "open"
      ? m.reports.noneOpen
      : status === "resolved"
        ? m.reports.noneResolved
        : m.reports.noneDismissed;
  return (
    <Section id="admin-reports" title={m.reports.title} hint={m.reports.hint}>
      <ToggleButtonGroup
        aria-label={m.reports.tabs}
        selectionMode="single"
        disallowEmptySelection
        selectedKeys={[status]}
        onSelectionChange={(keys) => {
          const [next] = Array.from(keys);
          const found = STATUSES.find((s) => s === next);
          if (found !== undefined) {
            setStatus(found);
            setLimit(PAGE);
          }
        }}
        className="flex flex-wrap gap-1"
      >
        {STATUSES.map((s) => (
          <ToggleButton key={s} id={s} className={toggleChipClass}>
            {label(s)}
          </ToggleButton>
        ))}
      </ToggleButtonGroup>
      {read.error !== null && <ReadFailed error={read.error} onRetry={read.reload} />}
      {list === undefined ? (
        read.error === null && (
          <div aria-busy="true" className="flex flex-col gap-3">
            <Skeleton className="h-32 w-full" />
            <Skeleton className="h-32 w-full" />
          </div>
        )
      ) : list.cases.length === 0 ? (
        <p className="text-sm text-ink-muted">{none}</p>
      ) : (
        <ul className="flex flex-col gap-3">
          {list.cases.map((c) => (
            <li key={c.id}>
              <CaseCard report={c} named={namedBy(list)} onChanged={read.reload} />
            </li>
          ))}
        </ul>
      )}
      {list !== undefined && list.cases.length >= limit && limit < MAX_LISTED && (
        <Button
          onPress={() => {
            setLimit((n) => Math.min(n + PAGE, MAX_LISTED));
          }}
          className={secondaryButtonClass + " self-start"}
        >
          {m.reports.showMore}
        </Button>
      )}
    </Section>
  );
}

/** Someone a case names: their picture and name, or a placeholder for an account now gone. */
function Person({ user }: { user: User | undefined }) {
  const m = useMessages();
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5">
      <Avatar name={name} iconId={user?.icon ?? null} size="xs" />
      <span className="truncate font-medium">{name}</span>
      {user?.bot === true && <BotBadge />}
    </span>
  );
}

/** One case: what was reported, its reports, and what can be done about it, or was. */
function CaseCard({
  report: c,
  named,
  onChanged,
}: {
  report: ReportCase;
  named: Named;
  onChanged: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const timeFormat = useDateFormat(TIME);
  const aspectList = useAspectList();
  const [resolving, setResolving] = useState(false);
  const [showContext, setShowContext] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const subject = named.users.get(c.subject);
  const subjectName = subject === undefined ? m.unknownUser : displayNameOf(subject);
  const reported = c.message == null ? undefined : named.messages.get(c.message);
  const aspects = Array.from(new Set(c.reports.flatMap((r) => r.aspects)));
  const snapshot = [...c.reports].reverse().find((r) => r.profile != null)?.profile ?? null;
  const time = (at: string) => timeFormat.format(new Date(at));
  const closer = c.closedBy == null ? undefined : named.users.get(c.closedBy);
  const closerName = closer === undefined ? m.unknownUser : displayNameOf(closer);

  const setDismissed = (dismissed: boolean) => {
    setPending(true);
    setError(null);
    sync.admin.setReportCaseDismissed(c.id, dismissed).then(
      () => {
        toast(dismissed ? m.reports.dismissedToast : m.reports.restoredToast);
        setPending(false);
        onChanged();
      },
      (e: unknown) => {
        setError(e instanceof ApiProblemError ? e.message : String(e));
        setPending(false);
      },
    );
  };

  return (
    <article
      aria-labelledby={`case-${c.id}`}
      className="flex flex-col gap-3 rounded-lg border border-line bg-surface p-3"
    >
      <header className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="rounded border border-line px-1.5 text-xs text-ink-muted">
          {c.kind === "message" ? m.reports.kindMessage : m.reports.kindProfile}
        </span>
        <h3 id={`case-${c.id}`} className="flex min-w-0 items-center gap-1.5 text-sm">
          <Person user={subject} />
        </h3>
        {c.subjectBanned && (
          <span className="rounded border border-danger/40 px-1.5 text-xs text-danger">
            {m.reports.bannedTag}
          </span>
        )}
        <span className="ms-auto text-xs text-ink-muted">
          {c.reports.length === 1
            ? m.reports.oneReport
            : format(m.reports.reportCount, { count: String(c.reports.length) })}
          {" · "}
          {format(m.reports.lastReported, { time: time(c.lastReportedAt) })}
        </span>
      </header>

      {c.kind === "message" ? (
        reported === undefined ? (
          <p className="text-sm text-ink-faint italic">{m.reports.embedDeleted}</p>
        ) : (
          <div className="flex flex-col gap-1">
            <Where message={reported} named={named} />
            <EmbeddedMessage
              message={reported.message}
              community={undefined}
              deletedAt={reported.deletedAt ?? null}
            />
            <Button
              onPress={() => {
                setShowContext((shown) => !shown);
              }}
              className="self-start text-sm text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              {showContext ? m.reports.hideContext : m.reports.showContext}
            </Button>
            {showContext && (
              <ContextView
                caseId={c.id}
                reported={reported.message.id}
                dm={isDmOf(reported, named)}
              />
            )}
          </div>
        )
      ) : (
        <div className="grid gap-2 sm:grid-cols-2">
          {snapshot !== null && (
            <div className="flex flex-col gap-1">
              <span className="text-xs font-semibold text-ink-faint uppercase">
                {m.reports.profileThen}
              </span>
              <ProfileSnapshotCard snapshot={snapshot} aspects={aspects} />
            </div>
          )}
          {subject !== undefined && (
            <div className="flex flex-col gap-1">
              <span className="text-xs font-semibold text-ink-faint uppercase">
                {m.reports.profileNow}
              </span>
              <ProfileSnapshotCard
                snapshot={{
                  name: subject.name,
                  displayName: subject.displayName ?? null,
                  icon: subject.icon ?? null,
                  status: subject.status ?? null,
                  bio: subject.bio ?? null,
                  pronouns: subject.pronouns ?? null,
                }}
                aspects={[]}
              />
            </div>
          )}
        </div>
      )}

      <ul className="flex flex-col gap-2 border-t border-line pt-2">
        {c.reports.map((r) => (
          <li key={r.id} className="flex flex-col gap-0.5 text-sm">
            <span className="flex flex-wrap items-center gap-x-2">
              <Person user={named.users.get(r.reporter)} />
              <span className="rounded bg-surface-sunken px-1.5 text-xs">
                {named.categories.get(r.category) ?? ""}
              </span>
              {r.aspects.length > 0 && (
                <span className="text-xs text-danger">{aspectList(r.aspects)}</span>
              )}
              <span className="text-xs text-ink-faint">{time(r.createdAt)}</span>
            </span>
            {r.explanation != null && (
              <p className="break-words whitespace-pre-wrap text-ink-muted">{r.explanation}</p>
            )}
          </li>
        ))}
      </ul>

      <footer className="flex flex-wrap items-center gap-2 border-t border-line pt-2">
        {c.status === "resolved" && (
          <div className="flex flex-col gap-0.5 text-sm">
            <span className="text-ink-muted">
              {format(m.reports.resolvedBy, { name: closerName, time: time(c.closedAt ?? "") })}
            </span>
            {c.resolution?.warning != null && (
              <span>{format(m.reports.didWarn, { text: c.resolution.warning })}</span>
            )}
            {c.resolution?.ban != null && (
              <span>
                {c.resolution.ban.reason == null
                  ? m.reports.didBan
                  : format(m.reports.didBanReason, { reason: c.resolution.ban.reason })}
              </span>
            )}
            {c.resolution?.deletedMessage === true && <span>{m.reports.didDelete}</span>}
            {(c.resolution?.reset?.length ?? 0) > 0 && (
              <span>
                {format(m.reports.didReset, { aspects: aspectList(c.resolution?.reset ?? []) })}
              </span>
            )}
          </div>
        )}
        {c.status === "dismissed" && (
          <span className="text-sm text-ink-muted">
            {format(m.reports.dismissedBy, { name: closerName, time: time(c.closedAt ?? "") })}
          </span>
        )}
        {c.status !== "resolved" && !c.mayAct && (
          <p className="text-sm text-ink-muted">{m.reports.notYours}</p>
        )}
        {c.status === "open" && c.mayAct && (
          <>
            <Button
              onPress={() => {
                setResolving(true);
              }}
              className="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-contrast outline-none hover:bg-accent-strong pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              {m.reports.takeAction}
            </Button>
            <Button
              isDisabled={pending}
              onPress={() => {
                setDismissed(true);
              }}
              className={secondaryButtonClass}
            >
              {m.reports.dismiss}
            </Button>
            <ResolveDialog
              report={c}
              subject={subject}
              subjectName={subjectName}
              aspects={aspects}
              isOpen={resolving}
              onOpenChange={setResolving}
              onResolved={onChanged}
            />
          </>
        )}
        {c.status === "dismissed" && c.mayAct && (
          <Button
            isDisabled={pending}
            onPress={() => {
              setDismissed(false);
            }}
            className={secondaryButtonClass + " ms-auto"}
          >
            {m.reports.restore}
          </Button>
        )}
        {error !== null && (
          <p role="alert" className="w-full text-sm text-danger">
            {error}
          </p>
        )}
      </footer>
    </article>
  );
}

/** Whether a reported message is in a DM, or a thread of one. */
function isDmOf(reported: ReviewedMessage, named: Named): boolean {
  const channel = named.channels.get(reported.message.channelId);
  return channel !== undefined && channel.community == null;
}

/** Where a reported message was posted: a community's channel, or a DM and its people. */
function Where({ message, named }: { message: ReviewedMessage; named: Named }) {
  const m = useMessages();
  const channel = named.channels.get(message.message.channelId);
  if (channel === undefined) {
    return null;
  }
  const parent =
    channel.parentChannel == null ? undefined : named.channels.get(channel.parentChannel);
  const place =
    channel.community == null
      ? (() => {
          const dm = parent ?? channel;
          const names = dm.recipients
            .map((id) => named.users.get(id))
            .flatMap((u) => (u === undefined ? [] : [displayNameOf(u)]));
          return names.length === 0
            ? m.reports.inDm
            : format(m.reports.inDmWith, { names: names.join(", ") });
        })()
      : [
          named.communities.get(channel.community)?.name,
          parent === undefined ? undefined : `#${parent.name}`,
          `#${channel.name}`,
        ]
          .filter((part) => part !== undefined && part !== "#")
          .join(" › ");
  return <p className="text-xs text-ink-muted">{format(m.reports.where, { place })}</p>;
}

/**
 * The conversation around a reported message, deleted messages marked, the reported one
 * outlined, with earlier and later messages a page at a time. A DM's is logged when read.
 */
function ContextView({ caseId, reported, dm }: { caseId: string; reported: string; dm: boolean }) {
  const m = useMessages();
  const sync = useSync();
  const [messages, setMessages] = useState<readonly ReviewedMessage[] | null>(null);
  const [more, setMore] = useState({ before: false, after: false });
  const [community, setCommunity] = useState<string | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  const take = useCallback(
    (context: ReportContext) => {
      sync.store.ingest({ users: context.users, attachments: context.attachments });
      const channel = context.channels.find((c) => c.id === context.channel);
      setCommunity(channel?.community ?? null);
      return context;
    },
    [sync],
  );

  useEffect(() => {
    let current = true;
    sync.admin.reportContext(caseId).then(
      (context) => {
        if (current) {
          take(context);
          setMessages(context.messages);
          setMore({ before: context.moreBefore, after: context.moreAfter });
        }
      },
      (e: unknown) => {
        if (current) {
          setError(e instanceof ApiProblemError ? e.message : String(e));
        }
      },
    );
    return () => {
      current = false;
    };
  }, [sync, caseId, take]);

  const page = (side: "before" | "after") => {
    const shown = messages ?? [];
    const edge = side === "before" ? shown[0] : shown[shown.length - 1];
    if (edge === undefined) {
      return;
    }
    setPending(true);
    sync.admin.reportContext(caseId, { [side]: edge.message.id }).then(
      (context) => {
        take(context);
        setMessages((now) =>
          side === "before"
            ? [...context.messages, ...(now ?? [])]
            : [...(now ?? []), ...context.messages],
        );
        setMore((now) =>
          side === "before"
            ? { ...now, before: context.moreBefore }
            : { ...now, after: context.moreAfter },
        );
        setPending(false);
      },
      (e: unknown) => {
        setError(e instanceof ApiProblemError ? e.message : String(e));
        setPending(false);
      },
    );
  };

  return (
    <section
      aria-label={m.reports.contextLabel}
      className="flex max-h-[32rem] flex-col gap-1 overflow-y-auto rounded-md border border-line bg-surface-sunken p-2"
      tabIndex={0}
    >
      {dm && <p className="text-xs text-ink-muted">{m.reports.contextDmNote}</p>}
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      {messages === null && error === null && (
        <div aria-busy="true" className="flex flex-col gap-2">
          <Skeleton className="h-12 w-full" />
          <Skeleton className="h-12 w-full" />
        </div>
      )}
      {more.before && (
        <Button
          isDisabled={pending}
          onPress={() => {
            page("before");
          }}
          className={secondaryButtonClass + " self-center py-0.5 text-xs"}
        >
          {m.reports.earlier}
        </Button>
      )}
      {messages?.map((kept) => (
        <div
          key={kept.message.id}
          className={
            kept.message.id === reported ? "rounded-md ring-2 ring-danger ring-offset-1" : ""
          }
        >
          <EmbeddedMessage
            message={kept.message}
            community={community}
            deletedAt={kept.deletedAt ?? null}
          />
        </div>
      ))}
      {more.after && (
        <Button
          isDisabled={pending}
          onPress={() => {
            page("after");
          }}
          className={secondaryButtonClass + " self-center py-0.5 text-xs"}
        >
          {m.reports.later}
        </Button>
      )}
    </section>
  );
}
