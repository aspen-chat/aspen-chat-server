import { ApiProblemError, isDm } from "@aspen/protocol";
import { PaneEdge, ResizablePane } from "@/features/layout/ResizablePane";
import { THREAD_PANEL } from "@/features/layout/paneSizes";
import { ChatsCircleIcon, XIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useBlocked, useChannel, useMessage, useSync, useSyncStatus } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { BlockedRun } from "@/features/messages/BlockedRun";
import { Composer } from "@/features/messages/Composer";
import { MessageItem } from "@/features/messages/MessageItem";
import { MessageList } from "@/features/messages/MessageList";
import { channelLink, type ChannelHome } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { CopyIdButton } from "@/features/layout/CopyId";
import { Toasts } from "@/features/layout/Toasts";

/**
 * A thread beside its channel: the message that started it, its replies, and a composer whose
 * checkbox also shows a reply in the parent channel. Opened from a link as well as from the
 * channel, so the thread, its starter, and its replies are each read when the cache lacks them.
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
  const navigate = useNavigate();
  const thread = useChannel(threadId);
  const parent = useChannel(parentId);
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

  const echoTarget =
    parent === undefined || isDm(parent) ? m.threads.thisConversation : `#${parent.name}`;
  const starter = thread?.starterMessage;
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
          <CopyIdButton id={threadId} thing="thread" />
        </header>
        {error !== null ? (
          <p role="alert" className="p-4 text-sm text-danger">
            {error}
          </p>
        ) : (
          <>
            <div className="relative flex min-h-0 flex-1 flex-col">
              {/* The starter heads the replies and scrolls with them, so a long one never
                  crowds them out of the panel. */}
              <MessageList
                channelId={threadId}
                home={home}
                highlightId={undefined}
                start={
                  <div className="border-b border-line pb-2">
                    {starterGone ? (
                      <p className="text-sm text-ink-faint italic">{m.threads.starterDeleted}</p>
                    ) : (
                      starter != null && <Starter id={starter} home={home} channelId={parentId} />
                    )}
                  </div>
                }
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
