import { ApiProblemError } from "@aspen/protocol";
import { PaneEdge, ResizablePane } from "@/features/layout/ResizablePane";
import { THREAD_PANEL } from "@/features/layout/paneSizes";
import { ChatsCircleIcon, XIcon } from "@phosphor-icons/react";
import { Navigate, useNavigate } from "@tanstack/react-router";
import { useEffect, useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import {
  useBlocked,
  useChannel,
  useChannelRemoved,
  useMessage,
  useSync,
  useSyncStatus,
} from "@/api/hooks";
import { useEchoTarget } from "@/features/threads/echoTarget";
import { FollowButton } from "@/features/threads/FollowButton";
import { Tooltip } from "@/features/layout/Tooltip";
import { BlockedRun } from "@/features/messages/BlockedRun";
import { Composer } from "@/features/messages/Composer";
import { MessageItem } from "@/features/messages/MessageItem";
import { MessageList } from "@/features/messages/MessageList";
import {
  channelLink,
  newThreadLink,
  threadLink,
  type ChannelHome,
} from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { CopyIdButton } from "@/features/layout/CopyId";
import { Toasts } from "@/features/layout/Toasts";

/**
 * A thread beside its channel: the message that started it, its replies, and a composer whose
 * checkbox also shows a reply in the parent channel. Opened from a link as well as from the
 * channel, so the thread, its starter, and its replies are each read when the cache lacks them.
 * Removed with the dropped first reply that made it, it gives way to `NewThreadPanel`.
 */
export function ThreadPanel({
  home,
  parentId,
  threadId,
}: {
  home: ChannelHome;
  parentId: string;
  threadId: string;
}) {
  const onePane = useOnePane();
  const m = useMessages();
  const sync = useSync();
  const status = useSyncStatus();
  const thread = useChannel(threadId);
  // Each is kept with the thread it concerns, so opening another thread starts clean.
  const [failure, setFailure] = useState<{ threadId: string; message: string } | null>(null);
  const [starterGoneFor, setStarterGoneFor] = useState<string | null>(null);
  const error = failure?.threadId === threadId ? failure.message : null;
  const starterGone = starterGoneFor === threadId;
  const live = status === "live";

  useEffect(() => {
    if (!live) {
      return;
    }
    let cancelled = false;
    sync
      .loadChannel(threadId)
      .then(async (loaded) => {
        if (loaded.starterMessage != null) {
          await sync.loadMessage(loaded.starterMessage).catch(() => {
            if (!cancelled) {
              setStarterGoneFor(threadId);
            }
          });
        }
        if (sync.store.messages(threadId) === undefined) {
          await sync.loadLatest(threadId);
        }
      })
      .catch((problem: unknown) => {
        if (!cancelled) {
          setFailure({
            threadId,
            message: problem instanceof ApiProblemError ? problem.message : m.threads.notFound,
          });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [sync, threadId, live, m]);

  const echoTarget = useEchoTarget(parentId);
  const starter = thread?.starterMessage;
  // The message it started from, kept once known: a thread removed with the dropped first reply
  // that made it gives way to that message's thread not made yet, where the reply waits to be
  // sent again.
  const [startedFrom, setStartedFrom] = useState<{ threadId: string; starter: string } | null>(
    null,
  );
  if (starter != null && (startedFrom?.threadId !== threadId || startedFrom.starter !== starter)) {
    setStartedFrom({ threadId, starter });
  }
  const knownStarter = starter ?? (startedFrom?.threadId === threadId ? startedFrom.starter : null);
  const removed = useChannelRemoved(threadId);
  const starterRecord = useMessage(knownStarter ?? "");
  if (
    removed &&
    knownStarter !== null &&
    starterRecord !== undefined &&
    starterRecord.thread == null
  ) {
    return <Navigate {...newThreadLink(home, parentId, knownStarter)} replace />;
  }
  return (
    <ThreadShell home={home} parentId={parentId} threadId={threadId}>
      {error !== null ? (
        <p role="alert" className="p-4 text-sm text-danger">
          {error}
        </p>
      ) : (
        <>
          <div className="relative flex min-h-0 flex-1 flex-col">
            {/* The starter heads the replies and scrolls with them, so a long one never
                  crowds them out of the panel; a short thread reads down from the panel's top,
                  as the thread not made yet does. */}
            <MessageList
              channelId={threadId}
              home={home}
              highlightId={undefined}
              fromTop
              start={
                <div className="border-b border-line pb-2">
                  {starterGone ? (
                    <p className="text-sm text-ink-faint italic">{m.threads.starterDeleted}</p>
                  ) : (
                    starter != null && <Starter id={starter} home={home} channelId={parentId} />
                  )}
                </div>
              }
              empty={<p className="py-2 text-sm text-ink-faint">{m.threads.noReplies}</p>}
            />
            {/* Toasts show over the channel's messages beside the panel; on a one-pane
                  screen the panel is the whole screen and shows its own. */}
            {onePane && <Toasts />}
          </div>
          <Composer
            key={threadId}
            channelId={threadId}
            placeholder={m.threads.placeholder}
            echoTarget={echoTarget}
          />
        </>
      )}
    </ThreadShell>
  );
}

/**
 * A thread of `starterId` beside its channel before the thread is made: the message it starts
 * from, and a composer whose first reply makes it (`Composer`'s `startsThreadOf`). Once the
 * message names its thread, made by that reply or by someone else's meanwhile, the thread opens
 * in its place.
 */
export function NewThreadPanel({
  home,
  parentId,
  starterId,
}: {
  home: ChannelHome;
  parentId: string;
  starterId: string;
}) {
  const onePane = useOnePane();
  const m = useMessages();
  const sync = useSync();
  const live = useSyncStatus() === "live";
  const starter = useMessage(starterId);
  const [missingFor, setMissingFor] = useState<string | null>(null);
  const echoTarget = useEchoTarget(parentId);
  const held = starter !== undefined;

  useEffect(() => {
    if (!live || held) {
      return;
    }
    let cancelled = false;
    sync.loadMessage(starterId).catch(() => {
      if (!cancelled) {
        setMissingFor(starterId);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [sync, starterId, live, held]);

  if (starter?.thread != null) {
    return <Navigate {...threadLink(home, parentId, starter.thread)} replace />;
  }
  return (
    <ThreadShell home={home} parentId={parentId} threadId={null}>
      {missingFor === starterId ? (
        <p role="alert" className="p-4 text-sm text-danger">
          {m.threads.starterNotFound}
        </p>
      ) : (
        <>
          <div className="relative flex min-h-0 flex-1 flex-col overflow-y-auto">
            <div className="border-b border-line pb-2">
              {held && <Starter id={starterId} home={home} channelId={parentId} />}
            </div>
            <p className="px-4 py-3 text-sm text-ink-faint">{m.threads.firstReply}</p>
            {onePane && <Toasts />}
          </div>
          <Composer
            key={starterId}
            channelId={parentId}
            startsThreadOf={starterId}
            placeholder={m.threads.placeholder}
            echoTarget={echoTarget}
          />
        </>
      )}
    </ThreadShell>
  );
}

/**
 * What a thread's panel is framed in, made or not: its heading, and a button closing it; for a
 * made thread, `threadId`, its bell and its id besides.
 */
function ThreadShell({
  home,
  parentId,
  threadId,
  children,
}: {
  home: ChannelHome;
  parentId: string;
  threadId: string | null;
  children: ReactNode;
}) {
  const onePane = useOnePane();
  const m = useMessages();
  const navigate = useNavigate();
  return (
    // Beside its channel it is complementary; alone on a phone it is the page's main content.
    <ResizablePane
      sizing={THREAD_PANEL}
      edge="start"
      label={m.layout.threadPanel}
      className="flex min-h-0 w-full"
    >
      <section
        role={onePane ? "main" : "complementary"}
        aria-label={m.threads.heading}
        className="motion-from-end relative flex min-h-0 w-full flex-col border-s border-line bg-surface"
      >
        <header className="flex items-center gap-2 border-b border-line px-4 py-3">
          <ChatsCircleIcon size={18} aria-hidden="true" className="text-ink-faint" />
          {onePane ? (
            <h1 className="flex-1 font-semibold">{m.threads.heading}</h1>
          ) : (
            <h2 className="flex-1 font-semibold">{m.threads.heading}</h2>
          )}
          {threadId !== null && <FollowButton threadId={threadId} />}
          <Tooltip text={m.threads.close}>
            <Button
              aria-label={m.threads.close}
              onPress={() => {
                void navigate(channelLink(home, parentId));
              }}
              className="tap-target rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              <XIcon size={18} aria-hidden="true" />
            </Button>
          </Tooltip>
          {threadId !== null && <CopyIdButton id={threadId} thing="thread" />}
        </header>
        {children}
        <PaneEdge />
      </section>
    </ResizablePane>
  );
}

/** The message that started the thread, collapsed like any other when its author is blocked. */
function Starter({ id, home, channelId }: { id: string; home: ChannelHome; channelId: string }) {
  const blocked = useBlocked(useMessage(id)?.author);
  const item = (messageId: string) => (
    <MessageItem
      id={messageId}
      home={home}
      channelId={channelId}
      parentId={null}
      highlighted={false}
      threadable={false}
    />
  );
  return blocked ? (
    <BlockedRun ids={[id]} lineOffset={null} highlightId={undefined} item={item} />
  ) : (
    item(id)
  );
}
