import {
  ACTIVITY_PAGE,
  ApiProblemError,
  type ActivityView,
  type Message,
  type Topic,
} from "@aspen/protocol";
import { CheckIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button, Tab, TabList, TabPanel, Tabs } from "react-aria-components";
import { SourceScope } from "@/api/deployments";
import { useEverywhere, useSources, type Source } from "@/api/everywhere";
import {
  useChannelRemoved,
  useLastRead,
  useMessageOnDemand,
  useMessageWindow,
  usePreference,
  useReadState,
  useSync,
} from "@/api/hooks";
import { ActivityFilters } from "@/features/activity/ActivityFilters";
import {
  ACTIVITY_HIDDEN,
  ACTIVITY_TAB,
  ACTIVITY_TABS,
  readFilter,
  shows,
  type ActivityTab,
} from "@/features/activity/filter";
import { PersonalPage } from "@/features/activity/PersonalPage";
import { foldThreads } from "@/features/activity/threads";
import { unreadEntries, type UnreadPlace } from "@/features/activity/unread";
import { LoadingLabel } from "@/features/layout/Skeleton";
import { toast } from "@/features/layout/toast";
import { Tooltip } from "@/features/layout/Tooltip";
import { Composer } from "@/features/messages/Composer";
import { channelLink, threadLink, type ChannelHome } from "@/features/messages/links";
import {
  ListedMessage,
  ListedMessageSkeleton,
  listedActionClass,
} from "@/features/messages/ListedMessage";
import { useChannelPlace, useMessagePlace } from "@/features/messages/place";
import { mergeResults, type SourceResults } from "@/features/search/merge";
import { useMessages } from "@/i18n/context";
import { format, type Messages } from "@/i18n/messages";
import { useDocumentTitle } from "@/features/layout/documentTitle";

/** How many of a thread's latest replies, or a channel's latest unread messages, its entry shows. */
const ENTRY_MESSAGES_SHOWN = 5;

/** How many entries the Unread tab shows at a time; each reads its channel's latest page. */
const UNREAD_PAGE = 10;

const tabClass =
  "cursor-default rounded-md px-2 py-1.5 text-sm md:px-3 outline-none hover:bg-surface-hover " +
  "selected:bg-accent-soft selected:text-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50";

const textButtonClass =
  "rounded-md px-3 py-1.5 text-sm text-accent outline-none hover:bg-surface-hover " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";

const jumpClass =
  "shrink-0 rounded px-1 text-xs text-accent outline-none hover:underline " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";

function tabName(m: Messages, tab: ActivityTab): string {
  switch (tab) {
    case "unread":
      return m.activity.tabUnread;
    case "mentions":
      return m.activity.tabMentions;
    case "dms":
      return m.activity.tabDms;
    case "threads":
      return m.activity.tabThreads;
    case "all":
      return m.activity.tabAll;
  }
}

/**
 * The activity feed, `/activity`: what is said where the reader reads, from every deployment
 * they use, newest first, under the tab chosen (`ACTIVITY_TAB`): what is unread, a channel or
 * thread to an entry (`UnreadTab`), or the messages of one of the server's views (`FeedTab`):
 * those that tag the reader, their DMs, replies in threads they follow, or everything. What it
 * shows is narrowed in its rail (`ActivityFilters`), and showing it marks nothing read.
 */
export function ActivityScreen() {
  const m = useMessages();
  useDocumentTitle(m.activity.heading);
  const sync = useSync();
  const tab = usePreference(ACTIVITY_TAB);
  return (
    <PersonalPage current="activity" rail={<ActivityFilters />}>
      <Tabs
        selectedKey={tab}
        onSelectionChange={(key) => {
          const chosen = ACTIVITY_TABS.find((known) => known === key);
          if (chosen !== undefined) {
            void sync.preferences.set(ACTIVITY_TAB, chosen);
          }
        }}
        className="flex flex-col gap-3"
      >
        <TabList aria-label={m.activity.tabs} className="flex flex-wrap gap-1 px-2">
          {ACTIVITY_TABS.map((known) => (
            <Tab key={known} id={known} className={tabClass}>
              {tabName(m, known)}
            </Tab>
          ))}
        </TabList>
        <TabPanel id={tab} className="flex flex-col gap-3 outline-none">
          {tab === "unread" ? <UnreadTab /> : <FeedTab key={tab} view={tab} />}
        </TabPanel>
      </Tabs>
    </PersonalPage>
  );
}

/** One deployment's part of the feed so far, newest first. */
interface Feed extends SourceResults<Message> {
  readonly source: Source;
  readonly problem: string | null;
}

/**
 * Every deployment's page of one view of the feed (only what is unread, with `unread`), as far
 * as the rail's filter shows it, with older pages on request (`more`). What arrives while it is
 * held joins the top of its deployment's feed if the view and the filter hold it. `feeds` is
 * `null` until the pages asked for have come; `reload` reads them afresh.
 */
function useFeeds(view: ActivityView, unread: boolean) {
  const sources = useSources();
  const hiddenList = usePreference(ACTIVITY_HIDDEN);
  const hidden = useMemo(() => new Set(hiddenList), [hiddenList]);
  // Each deployment's communities, which a filter leaving one out names the rest of.
  const communities = useEverywhere(["communities"], (all) =>
    all
      .map((source) =>
        source.sync.store
          .communities()
          .map((c) => c.id)
          .join(","),
      )
      .join(";"),
  );
  const [reloads, setReloads] = useState(0);
  // What the feed was read for: the deployments, the view, and the filter. Pages read for
  // anything else are dropped when they come, and the feed stands in skeleton until its own
  // arrive.
  const query = `${view}|${String(unread)}|${hiddenList.join(",")}|${communities}|${String(reloads)}`;
  const [read, setRead] = useState<{
    sources: readonly Source[];
    query: string;
    feeds: readonly Feed[];
  } | null>(null);
  const feeds = read?.sources === sources && read.query === query ? read.feeds : null;
  const setFeeds = (change: (held: readonly Feed[]) => readonly Feed[]) => {
    setRead((held) => (held === null ? held : { ...held, feeds: change(held.feeds) }));
  };
  const [loadingMore, setLoadingMore] = useState(false);
  const asked = useRef({ sources, query });
  const stillAsked = () => asked.current.sources === sources && asked.current.query === query;
  const filterOf = (source: Source) =>
    readFilter(
      hidden,
      source.domain,
      source.sync.store.communities().map((c) => c.id),
      view,
    );

  const page = async (source: Source, before: string | undefined, earlier: readonly Message[]) => {
    const filter = filterOf(source);
    const key = source.domain ?? "";
    if (filter === null) {
      return { key, source, messages: earlier, exhausted: true, problem: null };
    }
    try {
      const found = await source.sync.readActivity({
        ...filter,
        view,
        unread,
        ...(before === undefined ? {} : { before }),
      });
      return {
        key,
        source,
        messages: [...earlier, ...found],
        exhausted: found.length < ACTIVITY_PAGE,
        problem: null,
      };
    } catch (error) {
      return {
        key,
        source,
        messages: earlier,
        exhausted: true,
        problem: error instanceof ApiProblemError ? error.message : String(error),
      };
    }
  };

  useEffect(() => {
    asked.current = { sources, query };
    void Promise.all(sources.map((source) => page(source, undefined, []))).then((feeds) => {
      if (asked.current.sources === sources && asked.current.query === query) {
        setRead({ sources, query, feeds });
      }
    });
    // `page` reads the view and the filter `query` names.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sources, query]);

  useEffect(() => {
    const stops = sources.map((source) =>
      source.sync.onArrival((message) => {
        if (!source.sync.store.inActivity(message, view)) {
          return;
        }
        const channel = source.sync.store.channel(message.channelId);
        const place =
          channel?.parentChannel == null
            ? channel
            : source.sync.store.channel(channel.parentChannel);
        if (!shows(hidden, source.domain, place?.community ?? null)) {
          return;
        }
        setFeeds((held) =>
          held.map((feed) =>
            feed.source !== source || feed.messages.some((known) => known.id === message.id)
              ? feed
              : { ...feed, messages: [message, ...feed.messages] },
          ),
        );
      }),
    );
    return () => {
      for (const stop of stops) {
        stop();
      }
    };
  }, [sources, hidden, view]);

  const more = async () => {
    if (feeds === null) {
      return;
    }
    setLoadingMore(true);
    const paged = await Promise.all(
      feeds.map((feed) =>
        feed.exhausted
          ? Promise.resolve(feed)
          : page(feed.source, feed.messages.at(-1)?.id, feed.messages),
      ),
    );
    setLoadingMore(false);
    if (stillAsked()) {
      // What arrived while the pages were read stays at the top.
      setFeeds((held) =>
        paged.map((feed) => {
          const before = held.find((known) => known.source === feed.source);
          const arrived =
            before?.messages.filter(
              (message) => !feed.messages.some((known) => known.id === message.id),
            ) ?? [];
          return { ...feed, messages: [...arrived, ...feed.messages] };
        }),
      );
    }
  };

  return {
    sources,
    hidden,
    feeds,
    more,
    loadingMore,
    reload: () => {
      setReloads((count) => count + 1);
    },
    everythingHidden: sources.length > 0 && sources.every((source) => filterOf(source) === null),
  };
}

function FeedProblems({ feeds }: { feeds: readonly Feed[] | null }) {
  const m = useMessages();
  return feeds?.map(
    (feed) =>
      feed.problem !== null && (
        <p key={feed.key} role="alert" className="px-2 text-sm text-danger">
          {format(m.activity.failedOn, {
            domain: feed.source.domain ?? m.activity.thisServer,
            problem: feed.problem,
          })}
        </p>
      ),
  );
}

function FeedSkeleton() {
  const m = useMessages();
  return (
    <div aria-busy="true" className="flex flex-col gap-1">
      <LoadingLabel text={m.activity.loading} />
      {[0, 1, 2, 3].map((index) => (
        <ListedMessageSkeleton key={index} index={index} />
      ))}
    </div>
  );
}

function noneOf(m: Messages, view: ActivityView): string {
  switch (view) {
    case "mentions":
      return m.activity.noneMentions;
    case "dms":
      return m.activity.noneDms;
    case "threads":
      return m.activity.noneThreads;
    case "all":
    case "notifying":
      return m.activity.none;
  }
}

/**
 * A tab listing one view's messages, read or not, merged across deployments as search merges.
 * A thread's replies stand as one entry (`foldThreads`, `ThreadItem`).
 */
function FeedTab({ view }: { view: ActivityView }) {
  const m = useMessages();
  const { feeds, more, loadingMore, everythingHidden } = useFeeds(view, false);
  const bySource = new Map(feeds?.map((feed) => [feed.key, feed.source]));
  const entries =
    feeds === null
      ? []
      : foldThreads(
          mergeResults(feeds),
          (key, channelId) =>
            bySource.get(key)?.sync.store.channel(channelId)?.parentChannel != null,
        );
  return (
    <>
      <FeedProblems feeds={feeds} />
      {feeds === null ? (
        <FeedSkeleton />
      ) : entries.length === 0 ? (
        <p className="px-2 text-sm text-ink-muted">
          {everythingHidden ? m.activity.noneShown : noneOf(m, view)}
        </p>
      ) : (
        <ul aria-label={m.activity.heading} className="message-text flex flex-col gap-1">
          {entries.map(({ key, message, thread }) => {
            const source = bySource.get(key);
            if (source === undefined) {
              return null;
            }
            // A thread's entry is keyed by the thread, so what is being written in it stays
            // when a newer reply takes the entry's place in the feed.
            return thread ? (
              <SourceScope key={`${key}/thread/${message.channelId}`} source={source}>
                <ThreadItem
                  threadId={message.channelId}
                  newest={message.id}
                  domain={source.domain}
                />
              </SourceScope>
            ) : (
              <SourceScope key={`${key}/${message.id}`} source={source}>
                <FeedItem message={message} domain={source.domain} />
              </SourceScope>
            );
          })}
        </ul>
      )}
      {feeds?.some((feed) => !feed.exhausted) === true && (
        <Button
          onPress={() => {
            void more();
          }}
          isPending={loadingMore}
          className={"self-center " + textButtonClass}
        >
          {m.activity.more}
        </Button>
      )}
    </>
  );
}

/**
 * The Unread tab: every channel and DM with something unread, as each deployment's store holds
 * them, and every followed thread with unread replies, as each deployment's feed gives them
 * (`unreadEntries`), `UNREAD_PAGE` at a time. Each entry marks its own place read, and Mark all
 * read marks every channel the filter shows and every thread read so far, then reads the
 * threads again, since more may wait behind them.
 */
function UnreadTab() {
  const m = useMessages();
  const { sources, hidden, feeds, more, loadingMore, reload, everythingHidden } = useFeeds(
    "threads",
    true,
  );
  const [shown, setShown] = useState(UNREAD_PAGE);
  const [marking, setMarking] = useState(false);
  const unreadChannels = useEverywhere(["unread"], (all) =>
    all.flatMap((source) =>
      source.sync.store.unreadChannels().map((unread) => ({ domain: source.domain, ...unread })),
    ),
  );
  const channels = unreadChannels
    .filter((unread) => shows(hidden, unread.domain, unread.community))
    .map((unread): UnreadPlace => ({
      key: unread.domain ?? "",
      channelId: unread.channel,
      newest: unread.lastMessage,
    }));
  const listed = unreadEntries(channels, feeds ?? []);
  const bySource = new Map(sources.map((source) => [source.domain ?? "", source]));
  // Drawn again when a listed thread's position moves: a thread read since its replies were
  // (here, by answering in it, or on another device) is unread no longer.
  useEverywhere(
    listed.filter((entry) => entry.thread).map((entry): Topic => `read:${entry.channelId}`),
    () => ({}),
  );
  const entries = listed.filter((entry) => {
    const read = entry.thread
      ? bySource.get(entry.key)?.sync.store.threadRead(entry.channelId)
      : undefined;
    return read === undefined || entry.newest > read;
  });
  const moreToRead = feeds?.some((feed) => !feed.exhausted) === true;

  const markAll = async () => {
    setMarking(true);
    try {
      await Promise.all(
        sources.map((source) =>
          source.sync.markManyRead(
            entries
              .filter((entry) => entry.key === (source.domain ?? ""))
              .map((entry) => ({ channelId: entry.channelId, messageId: entry.newest })),
          ),
        ),
      );
    } catch (error) {
      toast(error instanceof ApiProblemError ? error.message : String(error));
    }
    setMarking(false);
    reload();
  };

  return (
    <>
      <FeedProblems feeds={feeds} />
      {feeds === null ? (
        <FeedSkeleton />
      ) : entries.length === 0 ? (
        <p className="px-2 text-sm text-ink-muted">
          {everythingHidden ? m.activity.noneShown : m.activity.noneUnread}
        </p>
      ) : (
        <>
          <Button
            onPress={() => {
              void markAll();
            }}
            isPending={marking}
            className={"self-end " + textButtonClass}
          >
            {m.activity.markAllRead}
          </Button>
          <ul aria-label={m.activity.tabUnread} className="message-text flex flex-col gap-1">
            {entries.slice(0, shown).map((entry) => {
              const source = bySource.get(entry.key);
              if (source === undefined) {
                return null;
              }
              return (
                <SourceScope
                  key={`${entry.key}/${entry.thread ? "thread" : "channel"}/${entry.channelId}`}
                  source={source}
                >
                  {entry.thread ? (
                    <ThreadItem
                      threadId={entry.channelId}
                      newest={entry.newest}
                      domain={source.domain}
                    />
                  ) : (
                    <ChannelItem
                      channelId={entry.channelId}
                      newest={entry.newest}
                      domain={source.domain}
                    />
                  )}
                </SourceScope>
              );
            })}
          </ul>
        </>
      )}
      {(entries.length > shown || moreToRead) && (
        <Button
          onPress={() => {
            setShown((count) => count + UNREAD_PAGE);
            if (entries.length <= shown + UNREAD_PAGE && moreToRead) {
              void more();
            }
          }}
          isPending={loadingMore}
          className={"self-center " + textButtonClass}
        >
          {m.activity.more}
        </Button>
      )}
    </>
  );
}

function UnreadMark() {
  const m = useMessages();
  return (
    <span className="flex items-center self-center" title={m.activity.unread}>
      <span className="forced-fill h-2 w-2 rounded-full bg-accent" aria-hidden="true" />
      <span className="sr-only">{m.activity.unread}</span>
    </span>
  );
}

/** Marks a channel, DM, or thread read up to `messageId`, from an entry that is unread. */
function MarkReadButton({ channelId, messageId }: { channelId: string; messageId: string }) {
  const m = useMessages();
  const sync = useSync();
  return (
    <Tooltip text={m.activity.markRead}>
      <Button
        aria-label={m.activity.markRead}
        onPress={() => {
          sync.markRead(channelId, messageId);
        }}
        className={listedActionClass}
      >
        <CheckIcon size={16} aria-hidden="true" />
      </Button>
    </Tooltip>
  );
}

/**
 * One message of the feed said outside any thread, in its deployment's scope: where it was said,
 * whether it is unread, a way to go to it, and, while it is unread, a way to mark its channel
 * read up to it. A message deleted since, or that the reader may no longer read, leaves the
 * feed.
 */
function FeedItem({ message: read, domain }: { message: Message; domain: string | null }) {
  const m = useMessages();
  const { message, missing } = useMessageOnDemand(read.id, true);
  const { home, link, where } = useMessagePlace(message ?? read, domain);
  const lastRead = useLastRead(read.channelId);
  if (missing) {
    return null;
  }
  const unread = lastRead === undefined || read.id > lastRead;
  return (
    <ListedMessage
      message={message}
      home={home}
      link={link}
      where={domain === null ? where : `${where} · ${format(m.search.onDomain, { domain })}`}
      mark={unread && <UnreadMark />}
      actions={unread && <MarkReadButton channelId={read.channelId} messageId={read.id} />}
    />
  );
}

/**
 * The messages of an entry's window to show, once the store's window of `channelId` reaches the
 * latest, which is read here when it does not (`AspenSync.loadLatest`): `null` meanwhile, and
 * `failed` when that read failed.
 */
function useLatestWindow(channelId: string) {
  const sync = useSync();
  const window = useMessageWindow(channelId);
  const current = window?.atLatest === true;
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!current) {
      sync.loadLatest(channelId).catch(() => {
        setFailed(true);
      });
    }
  }, [sync, channelId, current]);
  return { window: current ? window : null, failed };
}

/** The head of a channel's or thread's entry: whether it is unread, where it is, and its actions. */
function EntryHead({
  unread,
  place,
  count,
  link,
  channelId,
  newest,
}: {
  unread: boolean;
  place: string;
  count?: string;
  link: ReturnType<typeof channelLink> | ReturnType<typeof threadLink> | undefined;
  channelId: string;
  newest: string;
}) {
  const m = useMessages();
  return (
    <div className="flex items-baseline gap-2 px-2">
      {unread && <UnreadMark />}
      <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink-muted">{place}</span>
      {count !== undefined && <span className="shrink-0 text-xs text-ink-muted">{count}</span>}
      {link !== undefined && (
        <Link {...link} className={jumpClass}>
          {m.pins.jump}
        </Link>
      )}
      {unread && <MarkReadButton channelId={channelId} messageId={newest} />}
    </div>
  );
}

function EntryMessagesSkeleton({ label }: { label: string }) {
  return (
    <div aria-busy="true" className="ms-2 flex flex-col gap-1 border-s-2 border-line">
      <LoadingLabel text={label} />
      <ListedMessageSkeleton index={0} />
      <ListedMessageSkeleton index={1} />
    </div>
  );
}

/**
 * A channel or DM with something unread, in its deployment's scope, standing where its newest
 * unread message (`newest`) does: where it is, how many of its messages are unread as far as
 * its window reaches, a way to it, a way to mark it read, and its `ENTRY_MESSAGES_SHOWN` latest
 * unread messages, oldest first and followed live. If the window cannot be read, `newest` alone
 * is shown.
 */
function ChannelItem({
  channelId,
  newest,
  domain,
}: {
  channelId: string;
  newest: string;
  domain: string | null;
}) {
  const m = useMessages();
  const { place, home, where } = useChannelPlace(channelId, domain);
  const lastRead = useReadState(channelId)?.lastRead ?? "";
  const { window, failed } = useLatestWindow(channelId);
  const unread = window?.ids.filter((id) => id > lastRead) ?? null;
  const messages = unread?.slice(-ENTRY_MESSAGES_SHOWN) ?? (failed ? [newest] : null);
  const count =
    window === null || unread === null || unread.length === 0
      ? undefined
      : format(
          // Every message the window holds is unread and it has older ones: there may be more.
          window.hasOlder && unread.length === window.ids.length
            ? m.activity.unreadCountOrMore
            : m.activity.unreadCount,
          { count: unread.length },
        );
  return (
    <li className="flex flex-col gap-1 py-1.5">
      <EntryHead
        unread
        place={domain === null ? where : `${where} · ${format(m.search.onDomain, { domain })}`}
        {...(count === undefined ? {} : { count })}
        link={place === undefined ? undefined : channelLink(home, channelId)}
        channelId={channelId}
        newest={newest}
      />
      {messages === null ? (
        <EntryMessagesSkeleton label={m.activity.loadingUnread} />
      ) : (
        <ul
          aria-label={m.activity.latestUnread}
          className="ms-2 flex flex-col gap-1 border-s-2 border-line"
        >
          {messages.map((id) => (
            <EntryMessage key={id} id={id} home={home} />
          ))}
        </ul>
      )}
    </li>
  );
}

/**
 * A thread with replies in the feed, in its deployment's scope, standing where the newest of
 * them (`newest`) does: where the thread is, whether that reply is unread, a way to the thread,
 * a way to mark it read, its `ENTRY_MESSAGES_SHOWN` latest replies whoever wrote them, oldest
 * first and followed live, and its message box, so the reader answers without leaving. If the
 * thread's window cannot be read, `newest` alone is shown. A thread deleted since, or left with
 * no replies, leaves the feed.
 */
function ThreadItem({
  threadId,
  newest,
  domain,
}: {
  threadId: string;
  newest: string;
  domain: string | null;
}) {
  const m = useMessages();
  const { thread, home, where } = useChannelPlace(threadId, domain);
  const lastRead = useLastRead(threadId);
  const removed = useChannelRemoved(threadId);
  const { window, failed } = useLatestWindow(threadId);
  const replies = window?.ids.slice(-ENTRY_MESSAGES_SHOWN) ?? (failed ? [newest] : null);
  if (removed || replies?.length === 0) {
    return null;
  }
  return (
    <li className="flex flex-col gap-1 py-1.5">
      <EntryHead
        unread={lastRead === undefined || newest > lastRead}
        place={domain === null ? where : `${where} · ${format(m.search.onDomain, { domain })}`}
        link={
          thread?.parentChannel == null
            ? undefined
            : threadLink(home, thread.parentChannel, threadId)
        }
        channelId={threadId}
        newest={newest}
      />
      {replies === null ? (
        <EntryMessagesSkeleton label={m.activity.loadingReplies} />
      ) : (
        <ul
          aria-label={m.activity.latestReplies}
          className="ms-2 flex flex-col gap-1 border-s-2 border-line"
        >
          {replies.map((id) => (
            <EntryMessage key={id} id={id} home={home} />
          ))}
        </ul>
      )}
      <div className="flex flex-col px-2">
        <Composer key={threadId} channelId={threadId} placeholder={m.activity.replyPlaceholder} />
      </div>
    </li>
  );
}

/** One of the messages an entry shows; one deleted since leaves it. */
function EntryMessage({ id, home }: { id: string; home: ChannelHome }) {
  const { message, missing } = useMessageOnDemand(id, true);
  return missing ? null : <ListedMessage message={message} home={home} link={undefined} />;
}
