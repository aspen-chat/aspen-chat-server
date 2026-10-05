import {
  ApiProblemError,
  type LoggedChannel,
  type LoggedMessage,
  type ModerationEntry,
} from "@aspen/protocol";
import { Link } from "@tanstack/react-router";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import { useSync, useUser, useUserLoading, useIdWizard } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { Cell, Table } from "@/features/admin/FleetHealth";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { UserMention } from "@/features/messages/Mention";
import {
  channelLink,
  communityLink,
  messageLink,
  useDomain,
  type ChannelHome,
} from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { formatNodes } from "@/i18n/formatNodes";
import { format, type Messages } from "@/i18n/messages";
import { PersonName } from "@/features/users/PersonName";
import { CopyIdButton } from "@/features/layout/CopyId";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/** A page of the log is what the server gives by default. */
const PAGE = 50;

/**
 * The moderation log, newest first, a page at a time: each use of Moderate any community that a
 * community's own permissions would not have allowed, and every reading of a DM by someone not
 * in it. It is how the deployment's administrators oversee its moderators. Everything an entry
 * names is shown by name, from what the server resolved (`details`): people as chips that open
 * their cards, communities, channels, DMs, and messages as links while they stand, and the
 * names of those since deleted; only what no name was found for shows as its id.
 */
export function ModerationLog() {
  const timeFormat = useDateFormat(TIME);
  const m = useMessages();
  const sync = useSync();
  const wizard = useIdWizard();
  const [entries, setEntries] = useState<readonly ModerationEntry[]>([]);
  const [complete, setComplete] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(
    (before: string | undefined, isCurrent: () => boolean) => {
      sync.admin.moderationLog(before).then(
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
            ...(wizard ? [{ content: m.bots.idColumn }] : []),
          ]}
          numeric={[]}
          dimmed={loading && entries.length > 0}
          skeletonRows={loading && entries.length === 0 ? 5 : 0}
        >
          {entries.map((entry) => (
            <tr key={entry.id}>
              <Cell>{timeFormat.format(new Date(entry.at))}</Cell>
              <Cell>
                <Actor userId={entry.actor ?? undefined} />
              </Cell>
              <Cell>
                <span className="block">{actionName(m, entry.action)}</span>
                <Subject entry={entry} />
              </Cell>
              <Cell>
                <Place entry={entry} />
              </Cell>
              {wizard && (
                <Cell>
                  <CopyIdButton id={entry.id} thing="logEntry" />
                </Cell>
              )}
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
  return userId === undefined ? <>{m.admin.logAccountGone}</> : <Person id={userId} />;
}

/**
 * Someone the log names: a chip that opens their card once their record is read, and their id
 * until then, or when it cannot be.
 */
function Person({ id }: { id: string }) {
  const user = useUser(id);
  const loading = useUserLoading(id);
  if (user !== undefined) {
    return <UserMention id={id} chip />;
  }
  return loading ? <PersonName id={id} /> : <BareId id={id} />;
}

/** The last resort, an id no name could be found for. */
function BareId({ id }: { id: string }) {
  return <code className="font-mono text-xs break-all">{id}</code>;
}

const linkClass =
  "text-accent underline underline-offset-2 outline-none hover:decoration-2 focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * Where it was done: the community and the channel, each a link while it stands and its name,
 * marked deleted, once it does not; a DM is named by its people.
 */
function Place({ entry }: { entry: ModerationEntry }) {
  const m = useMessages();
  const domain = useDomain();
  const { community, channel } = entry.details;
  const parts: ReactNode[] = [];
  if (entry.community != null) {
    parts.push(
      community == null ? (
        <BareId id={entry.community} />
      ) : community.deleted ? (
        format(m.admin.logDeleted, { name: community.name })
      ) : (
        <Link {...communityLink(domain, entry.community)} className={linkClass}>
          {community.name}
        </Link>
      ),
    );
  }
  if (entry.channel != null) {
    parts.push(
      channel == null ? (
        <BareId id={entry.channel} />
      ) : (
        <ChannelPlace
          id={entry.channel}
          channel={channel}
          home={{ domain, community: entry.community ?? null }}
          gone={community?.deleted === true}
        />
      ),
    );
  }
  if (parts.length === 0) {
    return null;
  }
  return (
    <span className="flex flex-wrap items-baseline gap-x-1">
      {parts.map((part, index) => (
        <span key={index} className="flex items-baseline gap-x-1">
          {index > 0 && (
            <span aria-hidden="true" className="text-ink-faint">
              ›
            </span>
          )}
          {part}
        </span>
      ))}
    </span>
  );
}

/** A channel or DM the log names; `gone` when its community is deleted, taking it along. */
function ChannelPlace({
  id,
  channel,
  home,
  gone,
}: {
  id: string;
  channel: LoggedChannel;
  home: ChannelHome;
  gone: boolean;
}) {
  const m = useMessages();
  const isDm = channel.ty === "dm" || channel.ty === "groupDm";
  const people = channel.recipients.map((user, index) => (
    <span key={user}>
      {index > 0 && ", "}
      <Person id={user} />
    </span>
  ));
  const name = isDm
    ? formatNodes(channel.ty === "dm" ? m.admin.logDm : m.admin.logGroupDm, {
        people: <>{people}</>,
      })
    : `#${channel.name}`;
  if (channel.deleted || gone) {
    return <>{isDm ? name : format(m.admin.logDeleted, { name: `#${channel.name}` })}</>;
  }
  // A DM is named by chips that open cards, so the link is the word before them.
  return isDm ? (
    <span>
      <Link {...channelLink(home, id)} className={linkClass}>
        {m.admin.logOpenDm}
      </Link>{" "}
      {name}
    </span>
  ) : (
    <Link {...channelLink(home, id)} className={linkClass}>
      {name}
    </Link>
  );
}

/** What else it was done to, beyond the place: a person, a message, and what was taken from it. */
function Subject({ entry }: { entry: ModerationEntry }) {
  const m = useMessages();
  const domain = useDomain();
  const { details } = entry;
  const message =
    details.message == null ? null : (
      <MessageRef message={details.message} home={{ domain, community: entry.community ?? null }} />
    );
  const author = details.message == null ? null : <Person id={details.message.author} />;
  let line: ReactNode = null;
  if (message !== null && details.emoji != null && details.user != null) {
    line = formatNodes(m.admin.logSubjectReaction, {
      emoji: details.emoji,
      name: <Person id={details.user} />,
      message,
      author,
    });
  } else if (message !== null && details.attachment != null) {
    line = formatNodes(m.admin.logSubjectAttachment, { file: details.attachment, message, author });
  } else if (message !== null && details.writeIn != null) {
    line = formatNodes(m.admin.logSubjectWriteIn, { text: details.writeIn, message, author });
  } else if (message !== null) {
    line = formatNodes(m.admin.logSubjectMessage, { message, author });
  } else if (details.user != null) {
    line = <Person id={details.user} />;
  } else if (details.renamedTo != null) {
    line = format(m.admin.logRenamedTo, { name: details.renamedTo });
  } else if (entry.subject != null) {
    line = <BareId id={entry.subject} />;
  }
  return line === null ? null : (
    <span className="block text-ink-muted first-letter:uppercase">{line}</span>
  );
}

/** A message the log names: a link to it in its channel, or words saying it was deleted. */
function MessageRef({ message, home }: { message: LoggedMessage; home: ChannelHome }) {
  const m = useMessages();
  if (message.deleted) {
    return <>{m.admin.logDeletedMessage}</>;
  }
  return (
    <Link {...messageLink(home, message.channel, message.id)} className={linkClass}>
      {m.admin.logMessage}
    </Link>
  );
}
