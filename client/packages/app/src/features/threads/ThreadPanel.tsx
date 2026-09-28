import { ApiProblemError, isDm } from "@aspen/protocol";
import { ChatsCircleIcon, XIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useChannel, useSync, useSyncStatus } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { Composer } from "@/features/messages/Composer";
import { MessageItem } from "@/features/messages/MessageItem";
import { MessageList } from "@/features/messages/MessageList";
import { channelLink, type ChannelHome } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";

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
    <aside
      aria-label={m.threads.heading}
      className="flex min-h-0 w-full flex-col border-l border-line bg-surface md:w-96"
    >
      <header className="flex items-center gap-2 border-b border-line px-4 py-3">
        <ChatsCircleIcon size={18} aria-hidden="true" className="text-ink-faint" />
        <h2 className="flex-1 font-semibold">{m.threads.heading}</h2>
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
      </header>
      {error !== null ? (
        <p role="alert" className="p-4 text-sm text-danger">
          {error}
        </p>
      ) : (
        <>
          <div className="border-b border-line px-2 py-2">
            {starterGone ? (
              <p className="px-2 text-sm text-ink-faint italic">{m.threads.starterDeleted}</p>
            ) : (
              starter != null && (
                <MessageItem
                  id={starter}
                  home={home}
                  channelId={parentId}
                  parentId={null}
                  highlighted={false}
                  threadable={false}
                />
              )
            )}
          </div>
          <MessageList channelId={threadId} home={home} highlightId={undefined} />
          <Composer
            channelId={threadId}
            placeholder={m.threads.placeholder}
            echoTarget={echoTarget}
          />
        </>
      )}
    </aside>
  );
}
