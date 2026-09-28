import {
  HeadphonesIcon,
  MicrophoneIcon,
  MicrophoneSlashIcon,
  PhoneDisconnectIcon,
  SpeakerSlashIcon,
} from "@phosphor-icons/react";
import { Button } from "react-aria-components";
import { useChannel, useSync, useVoiceCall } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { ShareControl } from "@/features/voice/ShareControl";
import { useMessages } from "@/i18n/context";

const buttonClass =
  "rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover " +
  "focus-visible:ring-2 focus-visible:ring-accent/50 pointer-coarse:p-2.5";

/** The call the user is in, above their user bar: where they are, and mute, deafen, leave. */
export function CallBar() {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const channel = useChannel(call.channelId ?? "");
  if (call.status === "idle" || call.channelId === null) {
    return null;
  }
  const status =
    call.status === "connected"
      ? m.voice.connected
      : call.status === "rejoining"
        ? m.voice.rejoining
        : call.status === "failed"
          ? call.errorKind === "microphone"
            ? m.voice.microphoneFailed
            : m.voice.failed
          : m.voice.joining;
  return (
    <div
      role="region"
      aria-label={m.voice.callBarLabel}
      className="flex items-center gap-1 border-t border-line px-3 py-2"
    >
      <span className="flex min-w-0 flex-1 flex-col">
        <span
          className={
            "truncate text-sm font-medium " + (call.status === "connected" ? "text-online" : "")
          }
        >
          {status}
        </span>
        <span className="truncate text-xs text-ink-muted">
          {call.status === "failed" ? (call.error ?? channel?.name ?? "") : (channel?.name ?? "")}
        </span>
      </span>
      <Tooltip text={call.muted ? m.voice.unmute : m.voice.mute}>
        <Button
          aria-label={call.muted ? m.voice.unmute : m.voice.mute}
          aria-pressed={call.muted}
          onPress={() => {
            sync.voice.setMuted(!call.muted);
          }}
          className={buttonClass + (call.muted ? " text-danger" : "")}
        >
          {call.muted ? (
            <MicrophoneSlashIcon size={18} aria-hidden="true" />
          ) : (
            <MicrophoneIcon size={18} aria-hidden="true" />
          )}
        </Button>
      </Tooltip>
      <Tooltip text={call.deafened ? m.voice.undeafen : m.voice.deafen}>
        <Button
          aria-label={call.deafened ? m.voice.undeafen : m.voice.deafen}
          aria-pressed={call.deafened}
          onPress={() => {
            sync.voice.setDeafened(!call.deafened);
          }}
          className={buttonClass + (call.deafened ? " text-danger" : "")}
        >
          {call.deafened ? (
            <SpeakerSlashIcon size={18} aria-hidden="true" />
          ) : (
            <HeadphonesIcon size={18} aria-hidden="true" />
          )}
        </Button>
      </Tooltip>
      {call.status === "connected" && <ShareControl variant="bar" />}
      <Tooltip text={m.voice.leave}>
        <Button
          aria-label={m.voice.leave}
          onPress={() => {
            sync.voice.leave();
          }}
          className={buttonClass + " text-danger"}
        >
          <PhoneDisconnectIcon size={18} aria-hidden="true" />
        </Button>
      </Tooltip>
    </div>
  );
}
