import { ACTIVITY_PAGE, ApiProblemError, type Message } from "@aspen/protocol";
import { Link } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "react-aria-components";
import { SourceScope } from "@/api/deployments";
import { useEverywhere, useSources, type Source } from "@/api/everywhere";
import {
  useChannelRemoved,
  useLastRead,
  useMessageOnDemand,
  useMessageWindow,
  usePreference,
  useSync,
} from "@/api/hooks";
import { ActivityFilters } from "@/features/activity/ActivityFilters";
import {
  ACTIVITY_HIDDEN,
  ACTIVITY_UNREAD_ONLY,
  readFilter,
  shows,
} from "@/features/activity/filter";
import { PersonalPage } from "@/features/activity/PersonalPage";
import { foldThreads } from "@/features/activity/threads";
import { CompactCheckbox } from "@/features/layout/choices";
import { LoadingLabel } from "@/features/layout/Skeleton";
import { Composer } from "@/features/messages/Composer";
import type { ChannelHome } from "@/features/messages/links";
import { ListedMessage, ListedMessageSkeleton } from "@/features/messages/ListedMessage";
import { useMessagePlace } from "@/features/messages/place";
import { mergeResults, type SourceResults } from "@/features/search/merge";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** How many of a thread's latest replies its entry in the feed shows. */
const THREAD_REPLIES_SHOWN = 5;

/** One deployment's part of the feed so far, newest first. */
interface Feed extends SourceResults<Message> {
  readonly source: Source;
  readonly problem: string | null;
}

/**
 * The activity feed, `/activity`: every message that tells the reader of itself (by the rule
 * their notifications follow, and every reply in a thread they follow), from every deployment
 * they use, newest first, merged as search merges, with older pages on request. A thread's
 * replies stand as one entry (`foldThreads`, `ThreadItem`). What arrives while it is open joins
 * the top. What it shows is chosen in its rail (`ActivityFilters`), and it can show only what is
 * unread; opening it marks nothing read.
 */
export function ActivityScreen() {
  const m = useMessages();
  const sync = useSync();
  const sources = useSources();
  const hiddenList = usePreference(ACTIVITY_HIDDEN);
  const unreadOnly = usePreference(ACTIVITY_UNREAD_ONLY);
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
  // What the feed was read for: the deployments and the filter. Pages read for anything else
  // are dropped when they come, and the feed stands in skeleton until its own arrive.
  const query = `${hiddenList.join(",")}|${String(unreadOnly)}|${communities}`;
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

  const page = async (source: Source, before: string | undefined, earlier: readonly Message[]) => {
    const filter = readFilter(
      hidden,
      source.domain,
      source.sync.store.communities().map((c) => c.id),
    );
    const key = source.domain ?? "";
    if (filter === null) {
      return { key, source, messages: earlier, exhausted: true, problem: null };
    }
    try {
      const found = await source.sync.readActivity({
        ...filter,
        unread: unreadOnly,
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
    // `page` reads the filter `query` names.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sources, query]);

  // What arrives and would notify joins the top of its deployment's feed.
  useEffect(() => {
    const stops = sources.map((source) =>
      source.sync.onNotify((message) => {
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
  }, [sources, hidden]);

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

  const bySource = new Map(feeds?.map((feed) => [feed.key, feed.source]));
  const entries =
    feeds === null
      ? []
      : foldThreads(
          mergeResults(feeds),
          (key, channelId) =>
            bySource.get(key)?.sync.store.channel(channelId)?.parentChannel != null,
        );
  const everythingHidden =
    sources.length > 0 &&
    sources.every(
      (source) =>
        readFilter(
          hidden,
          source.domain,
          source.sync.store.communities().map((c) => c.id),
        ) === null,
    );
  return (
    <PersonalPage current="activity" rail={<ActivityFilters />}>
      <div className="flex items-center justify-end px-2">
        <CompactCheckbox
          isSelected={unreadOnly}
          onChange={(on) => {
            void sync.preferences.set(ACTIVITY_UNREAD_ONLY, on);
          }}
        >
          {m.activity.unreadOnly}
        </CompactCheckbox>
      </div>
      {feeds?.map(
        (feed) =>
          feed.problem !== null && (
            <p key={feed.key} role="alert" className="px-2 text-sm text-danger">
              {format(m.activity.failedOn, {
                domain: feed.source.domain ?? m.activity.thisServer,
                problem: feed.problem,
              })}
            </p>
          ),
      )}
      {feeds === null ? (
        <div aria-busy="true" className="flex flex-col gap-1">
          <LoadingLabel text={m.activity.loading} />
          {[0, 1, 2, 3].map((index) => (
            <ListedMessageSkeleton key={index} index={index} />
          ))}
        </div>
      ) : entries.length === 0 ? (
        <p className="px-2 text-sm text-ink-muted">
          {everythingHidden
            ? m.activity.noneShown
            : unreadOnly
              ? m.activity.noneUnread
              : m.activity.none}
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
                <ThreadItem newest={message} domain={source.domain} />
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
          className="self-center rounded-md px-3 py-1.5 text-sm text-accent outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {m.activity.more}
        </Button>
      )}
    </PersonalPage>
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

/**
 * One message of the feed said outside any thread, in its deployment's scope: where it was said,
 * whether it is unread, and a way to go to it. A message deleted since, or that the reader may
 * no longer read, leaves the feed.
 */
function FeedItem({ message: read, domain }: { message: Message; domain: string | null }) {
  const m = useMessages();
  const { message, missing } = useMessageOnDemand(read.id, true);
  const { home, link, where } = useMessagePlace(message ?? read, domain);
  const lastRead = useLastRead(read.channelId);
  if (missing) {
    return null;
  }
  return (
    <ListedMessage
      message={message}
      home={home}
      link={link}
      where={domain === null ? where : `${where} · ${format(m.search.onDomain, { domain })}`}
      mark={(lastRead === undefined || read.id > lastRead) && <UnreadMark />}
    />
  );
}

/**
 * A thread with replies in the feed, in its deployment's scope, standing where the newest of
 * them (`newest`) does: where the thread is, whether that reply is unread, a way to the thread,
 * its `THREAD_REPLIES_SHOWN` latest replies whoever wrote them, oldest first and followed live,
 * and its message box, so the reader answers without leaving. The replies are the thread's
 * window (`AspenSync.loadLatest`), read here when the store's does not reach the latest; if that
 * read fails, `newest` alone is shown. A thread deleted since, or left with no replies, leaves
 * the feed.
 */
function ThreadItem({ newest, domain }: { newest: Message; domain: string | null }) {
  const m = useMessages();
  const sync = useSync();
  const threadId = newest.channelId;
  const { home, link, where } = useMessagePlace(newest, domain);
  const lastRead = useLastRead(threadId);
  const removed = useChannelRemoved(threadId);
  const window = useMessageWindow(threadId);
  const current = window?.atLatest === true;
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!current) {
      sync.loadLatest(threadId).catch(() => {
        setFailed(true);
      });
    }
  }, [sync, threadId, current]);
  const replies = current ? window.ids.slice(-THREAD_REPLIES_SHOWN) : failed ? [newest.id] : null;
  if (removed || replies?.length === 0) {
    return null;
  }
  const place = domain === null ? where : `${where} · ${format(m.search.onDomain, { domain })}`;
  return (
    <li className="flex flex-col gap-1 py-1.5">
      <div className="flex items-baseline gap-2 px-2">
        {(lastRead === undefined || newest.id > lastRead) && <UnreadMark />}
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink-muted">{place}</span>
        {link !== undefined && (
          <Link
            {...link}
            className="shrink-0 rounded px-1 text-xs text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            {m.pins.jump}
          </Link>
        )}
      </div>
      {replies === null ? (
        <div aria-busy="true" className="ms-2 flex flex-col gap-1 border-s-2 border-line">
          <LoadingLabel text={m.activity.loadingReplies} />
          <ListedMessageSkeleton index={0} />
          <ListedMessageSkeleton index={1} />
        </div>
      ) : (
        <ul
          aria-label={m.activity.latestReplies}
          className="ms-2 flex flex-col gap-1 border-s-2 border-line"
        >
          {replies.map((id) => (
            <ThreadReply key={id} id={id} home={home} />
          ))}
        </ul>
      )}
      <div className="flex flex-col px-2">
        <Composer key={threadId} channelId={threadId} placeholder={m.activity.replyPlaceholder} />
      </div>
    </li>
  );
}

/** One of a thread's latest replies in its entry; one deleted since leaves it. */
function ThreadReply({ id, home }: { id: string; home: ChannelHome }) {
  const { message, missing } = useMessageOnDemand(id, true);
  return missing ? null : <ListedMessage message={message} home={home} link={undefined} />;
}
