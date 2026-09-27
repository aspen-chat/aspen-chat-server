import { ApiProblemError, isDm, type Channel } from "@aspen/protocol";
import { Navigate, useParams } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { useChannel, useChannelRemoved, useSync, useSyncStatus } from "@/api/hooks";
import { ChannelHeader } from "@/features/channels/ChannelHeader";
import { DmHeader } from "@/features/dms/DmHeader";
import { useDmTitle } from "@/features/dms/useDmTitle";
import { Composer } from "@/features/messages/Composer";
import { MessageList } from "@/features/messages/MessageList";
import { threadLink, type ChannelHome } from "@/features/messages/links";
import { ThreadPanel } from "@/features/threads/ThreadPanel";
import { VoiceScreen } from "@/features/voice/VoiceScreen";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A channel's screen, in a community or among the caller's DMs: a text channel's or DM's
 * history and composer, or a voice channel's call. With a message id in the URL the history
 * opens around that message; otherwise it opens at the newest messages and follows new ones as
 * they arrive. With a thread id the thread opens beside it, in place of it on small screens.
 */
export function ChannelScreen() {
  const m = useMessages();
  const { communityId, channelId, messageId, threadId } = useParams({ strict: false });
  const home: ChannelHome = communityId ?? null;
  const sync = useSync();
  const status = useSyncStatus();
  const channel = useChannel(channelId ?? "");
  const removed = useChannelRemoved(channelId ?? "");
  const [loadError, setLoadError] = useState<string | null>(null);
  /** The channel id last found missing, so a different id is not taken for missing too. */
  const [missingId, setMissingId] = useState<string | null>(null);
  const missing = missingId !== null && missingId === channelId;
  const live = status === "live";

  // A DM the listing did not bring, from a link say, is read on its own; one the caller is not
  // in reads as missing. One that was held and removed while shown is gone, not unread.
  const held = channel !== undefined;
  useEffect(() => {
    if (!live || held || removed || channelId === undefined) {
      return;
    }
    sync.loadChannel(channelId).catch(() => {
      setMissingId(channelId);
    });
  }, [sync, channelId, live, held, removed]);

  // Every (re)bootstrap drops the loaded windows, so reload whenever the sync comes back live.
  // A message id that merely leaves the URL, as it does once the reader scrolls, is not a
  // reason to reload: the window already holds that part of the history and stays put.
  const readable = channel !== undefined && channel.ty !== "voice" && channel.ty !== "thread";
  useEffect(() => {
    if (!live || !readable || channelId === undefined) {
      return;
    }
    if (messageId === undefined && sync.store.messages(channelId) !== undefined) {
      return;
    }
    const load =
      messageId === undefined ? sync.loadLatest(channelId) : sync.loadAround(channelId, messageId);
    load.then(
      () => {
        setLoadError(null);
      },
      (error: unknown) => {
        setLoadError(error instanceof ApiProblemError ? error.message : String(error));
      },
    );
  }, [sync, channelId, messageId, live, readable]);

  if (channel === undefined || channelId === undefined) {
    return (
      <main className="flex flex-1 items-center justify-center p-6 text-ink-muted">
        {home === null && !missing && !removed
          ? m.loading
          : home === null
            ? m.dms.notFound
            : m.channelNotFound}
      </main>
    );
  }

  // A thread is shown beside its channel, so a link to one as a channel opens it there.
  if (channel.ty === "thread" && channel.parentChannel != null) {
    return <Navigate {...threadLink(home, channel.parentChannel, channel.id)} replace />;
  }

  if (channel.ty === "voice" && communityId !== undefined) {
    return <VoiceScreen channel={channel} communityId={communityId} />;
  }

  return (
    <div className="flex min-h-0 flex-1">
      <main
        className={`${threadId === undefined ? "flex" : "hidden md:flex"} min-h-0 min-w-0 flex-1 flex-col`}
      >
        {isDm(channel) ? (
          <DmHeader channel={channel} />
        ) : (
          <ChannelHeader communityId={communityId ?? ""} glyph="#" name={channel.name} />
        )}
        {loadError !== null && (
          <p role="alert" className="bg-danger-soft px-4 py-2 text-sm text-danger">
            {loadError}
          </p>
        )}
        <MessageList channelId={channelId} highlightId={messageId} home={home} />
        <ChannelComposer channel={channel} />
      </main>
      {threadId !== undefined && (
        <ThreadPanel home={home} parentId={channelId} threadId={threadId} />
      )}
    </div>
  );
}

function ChannelComposer({ channel }: { channel: Channel }) {
  const m = useMessages();
  const title = useDmTitle(channel);
  return (
    <Composer
      channelId={channel.id}
      placeholder={
        isDm(channel)
          ? format(m.dms.placeholder, { name: title })
          : format(m.messagePlaceholder, { channel: channel.name })
      }
    />
  );
}
