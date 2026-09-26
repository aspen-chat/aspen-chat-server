import { ApiProblemError } from "@aspen/protocol";
import { useParams } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { useChannel, useSync, useSyncStatus } from "@/api/hooks";
import { ChannelHeader } from "@/features/channels/ChannelHeader";
import { Composer } from "@/features/messages/Composer";
import { MessageList } from "@/features/messages/MessageList";
import { VoiceScreen } from "@/features/voice/VoiceScreen";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A channel's screen: a text channel's history and composer, or a voice channel's call. With a
 * message id in the URL the history opens around that message; otherwise it opens at the
 * newest messages and follows new ones as they arrive.
 */
export function ChannelScreen() {
  const m = useMessages();
  const { communityId, channelId, messageId } = useParams({ strict: false });
  const sync = useSync();
  const status = useSyncStatus();
  const channel = useChannel(channelId ?? "");
  const [loadError, setLoadError] = useState<string | null>(null);

  // Every (re)bootstrap drops the loaded windows, so reload whenever the sync comes back live.
  // A message id that merely leaves the URL, as it does once the reader scrolls, is not a
  // reason to reload: the window already holds that part of the history and stays put.
  const live = status === "live";
  const text = channel?.ty === "Text";
  useEffect(() => {
    if (!live || !text || channelId === undefined) {
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
  }, [sync, channelId, messageId, live, text]);

  if (channel === undefined || channelId === undefined || communityId === undefined) {
    return (
      <main className="flex flex-1 items-center justify-center p-6 text-ink-muted">
        {m.channelNotFound}
      </main>
    );
  }

  if (channel.ty === "Voice") {
    return <VoiceScreen channel={channel} communityId={communityId} />;
  }

  return (
    <main className="flex min-h-0 flex-1 flex-col">
      <ChannelHeader communityId={communityId} glyph="#" name={channel.name} />
      {loadError !== null && (
        <p role="alert" className="bg-danger-soft px-4 py-2 text-sm text-danger">
          {loadError}
        </p>
      )}
      <MessageList channelId={channelId} highlightId={messageId} communityId={communityId} />
      <Composer
        channelId={channelId}
        placeholder={format(m.messagePlaceholder, { channel: channel.name })}
      />
    </main>
  );
}
