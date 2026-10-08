import { ACTIVITY_PAGE, ApiProblemError, type Message } from "@aspen/protocol";
import { ChatCircleIcon } from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button, ToggleButton } from "react-aria-components";
import { SourceScope } from "@/api/deployments";
import { useEverywhere, useSources, type Source } from "@/api/everywhere";
import { useLastRead, useMessageOnDemand, usePreference, useSync } from "@/api/hooks";
import { ActivityFilters } from "@/features/activity/ActivityFilters";
import {
  ACTIVITY_HIDDEN,
  ACTIVITY_UNREAD_ONLY,
  readFilter,
  shows,
} from "@/features/activity/filter";
import { PersonalPage } from "@/features/activity/PersonalPage";
import { CompactCheckbox } from "@/features/layout/choices";
import { LoadingLabel } from "@/features/layout/Skeleton";
import { Tooltip } from "@/features/layout/Tooltip";
import { Composer } from "@/features/messages/Composer";
import {
  ListedMessage,
  ListedMessageSkeleton,
  listedActionClass,
} from "@/features/messages/ListedMessage";
import { useMessagePlace } from "@/features/messages/place";
import { mergeResults, type SourceResults } from "@/features/search/merge";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** One deployment's part of the feed so far, newest first. */
interface Feed extends SourceResults<Message> {
  readonly source: Source;
  readonly problem: string | null;
}

/**
 * The activity feed, `/activity`: every message that tells the reader of itself (by the rule
 * their notifications follow, and every reply in a thread they follow), from every deployment
 * they use, newest first, merged as search merges, with older pages on request. What arrives
 * while it is open joins the top. What it shows is chosen in its rail (`ActivityFilters`), and
 * it can show only what is unread; opening it marks nothing read.
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

  const merged = feeds === null ? [] : mergeResults(feeds);
  const bySource = new Map(feeds?.map((feed) => [feed.key, feed.source]));
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
      ) : merged.length === 0 ? (
        <p className="px-2 text-sm text-ink-muted">
          {everythingHidden
            ? m.activity.noneShown
            : unreadOnly
              ? m.activity.noneUnread
              : m.activity.none}
        </p>
      ) : (
        <ul aria-label={m.activity.heading} className="flex flex-col gap-1">
          {merged.map(({ key, message }) => {
            const source = bySource.get(key);
            return (
              source !== undefined && (
                <SourceScope key={`${key}/${message.id}`} source={source}>
                  <FeedItem message={message} domain={source.domain} />
                </SourceScope>
              )
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

/**
 * One message of the feed, in its deployment's scope: where it was said, whether it is unread,
 * a way to go to it, and, for a reply in a thread, a way to reply there without leaving. A
 * message deleted since, or that the reader may no longer read, leaves the feed.
 */
function FeedItem({ message: read, domain }: { message: Message; domain: string | null }) {
  const m = useMessages();
  const { message, missing } = useMessageOnDemand(read.id, true);
  const { home, link, where, thread } = useMessagePlace(message ?? read, domain);
  const lastRead = useLastRead(read.channelId);
  const [replying, setReplying] = useState(false);
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
      mark={
        unread && (
          <span className="flex items-center self-center" title={m.activity.unread}>
            <span className="forced-fill h-2 w-2 rounded-full bg-accent" aria-hidden="true" />
            <span className="sr-only">{m.activity.unread}</span>
          </span>
        )
      }
      actions={
        thread !== undefined && (
          <Tooltip text={replying ? m.activity.hideReply : m.activity.reply}>
            <ToggleButton
              aria-label={replying ? m.activity.hideReply : m.activity.reply}
              isSelected={replying}
              onChange={setReplying}
              className={listedActionClass + " selected:text-accent"}
            >
              <ChatCircleIcon size={16} aria-hidden="true" />
            </ToggleButton>
          </Tooltip>
        )
      }
    >
      {replying && thread !== undefined && (
        <div className="mt-1 flex flex-col">
          <Composer
            key={thread.id}
            channelId={thread.id}
            placeholder={m.activity.replyPlaceholder}
          />
        </div>
      )}
    </ListedMessage>
  );
}
