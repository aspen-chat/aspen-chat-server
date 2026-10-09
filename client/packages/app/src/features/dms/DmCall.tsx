import type { Channel, ChannelVoice } from "@aspen/protocol";
import { PhoneIcon } from "@phosphor-icons/react";
import { useContext, useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useChannelCan, useChannelVoice, useMe, useSync, useVoiceCall } from "@/api/hooks";
import { HomeSyncContext } from "@/api/syncContext";
import { useNow } from "@/features/layout/useNow";
import { loopSound } from "@/features/notifications/sounds";
import { voiceOutputDevice } from "@/features/settings/audioDevices";
import { Tooltip } from "@/features/layout/Tooltip";
import { ShareControl } from "@/features/voice/ShareControl";
import { CallStage } from "@/features/voice/VoiceScreen";
import { useMessages } from "@/i18n/context";

/**
 * A DM's call, above its messages, while one is under way there or the user is joining or in
 * it: the same call a voice channel holds (`CallStage`), with the share control while the user
 * is in it, and the dial tone while it rings someone. It never takes more than about half the
 * height, so the conversation stays in view.
 * No one moderates it: its people hold no community permission.
 */
export function DmCall({ channel }: { channel: Channel }) {
  const m = useMessages();
  const call = useVoiceCall();
  const voice = useChannelVoice(channel.id);
  const [shareError, setShareError] = useState<string | null>(null);
  const here = call.status !== "idle" && call.channelId === channel.id;
  useDialTone(call.status === "connected" && here, voice);
  if (!here && voice.participants.length === 0) {
    return null;
  }
  return (
    <section
      aria-label={m.voice.dmCallLabel}
      className="flex max-h-[55%] min-h-0 shrink-0 flex-col border-b border-line bg-surface-sunken"
    >
      {shareError !== null && (
        <p role="alert" className="bg-danger-soft px-4 py-2 text-sm text-danger">
          {shareError}
        </p>
      )}
      <CallStage
        channel={channel}
        joinLabel={m.voice.joinCall}
        className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4"
        toolbarEnd={
          call.status === "connected" && here ? (
            // A narrow screen shares from the call bar instead.
            <div className="hidden md:flex">
              <ShareControl variant="panel" onError={setShareError} />
            </div>
          ) : undefined
        }
      />
    </section>
  );
}

/**
 * Plays the dial tone while the user is in a DM's call and it is still ringing someone else,
 * through the voice chat's speaker; it stops when they answer, decline, or their ring runs out.
 */
function useDialTone(connected: boolean, voice: ChannelVoice) {
  const home = useContext(HomeSyncContext);
  const me = useMe();
  const now = useNow(500, connected && voice.rings.length > 0);
  const ringing =
    connected &&
    voice.rings.some(
      (ring) =>
        ring.user !== me?.id &&
        Date.parse(ring.until) > now &&
        !voice.participants.some((participant) => participant.user === ring.user),
    );
  useEffect(() => {
    if (!ringing || home === null) {
      return;
    }
    return loopSound("dialTone", voiceOutputDevice(home.preferences));
  }, [ringing, home]);
}

/**
 * The DM header's call button: it starts the DM's call, or joins the one under way, for anyone
 * who may (a block between the two people of a one-to-one DM leaves neither able). It is not
 * shown while the user is already joining or in this DM's call.
 */
export function DmCallButton({ channel, className }: { channel: Channel; className: string }) {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const voice = useChannelVoice(channel.id);
  const mayJoin = useChannelCan(channel.id, "joinVoice");
  if (!mayJoin || (call.status !== "idle" && call.channelId === channel.id)) {
    return null;
  }
  const label = voice.participants.length === 0 ? m.voice.startCall : m.voice.joinCall;
  return (
    <Tooltip text={label}>
      <Button
        aria-label={label}
        onPress={() => {
          void sync.voice.join(channel.id).catch(() => undefined);
        }}
        className={className}
      >
        <PhoneIcon size={20} aria-hidden="true" />
      </Button>
    </Tooltip>
  );
}
