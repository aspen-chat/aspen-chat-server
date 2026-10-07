import {
  HeadphonesIcon,
  MicrophoneIcon,
  MicrophoneSlashIcon,
  PhoneDisconnectIcon,
  SpeakerSlashIcon,
  VideoCameraIcon,
  VideoCameraSlashIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Button } from "react-aria-components";
import { isDm, type CameraFailure, type Channel } from "@aspen/protocol";
import { useChannel, useSync, useVoiceCall } from "@/api/hooks";
import { useDmTitle } from "@/features/dms/useDmTitle";
import { channelLink, useDomain } from "@/features/messages/links";
import { Tooltip } from "@/features/layout/Tooltip";
import { ShareControl } from "@/features/voice/ShareControl";
import { useMessages } from "@/i18n/context";
import { useNumberFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

/** A wait in whole seconds, spelled out ("5 seconds") in the app's language. */
const SECONDS: Intl.NumberFormatOptions = { style: "unit", unit: "second", unitDisplay: "long" };

const buttonClass =
  "rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover " +
  "focus-visible:ring-2 focus-visible:ring-accent/50 pointer-coarse:p-2.5";

/**
 * The call the user is in, above their user bar: where they are (a voice channel, or a DM's
 * people), which opens that room, and mute, deafen, camera, share, leave.
 */
export function CallBar() {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const channel = useChannel(call.channelId ?? "");
  const seconds = useNumberFormat(SECONDS);
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
    <div role="region" aria-label={m.voice.callBarLabel} className="border-t border-line px-3 py-2">
      <div className="flex items-center gap-1">
        <CallPlaceLink channel={channel}>
          <span
            className={
              "truncate text-sm font-medium " + (call.status === "connected" ? "text-online" : "")
            }
          >
            {status}
          </span>
          <span className="truncate text-xs text-ink-muted">
            {call.status === "failed" && call.error !== null ? (
              call.errorKind === "refused" && call.retryAfterSeconds !== null ? (
                format(m.voice.tooFast, { wait: seconds.format(call.retryAfterSeconds) })
              ) : (
                call.error
              )
            ) : channel === undefined ? null : (
              <CallPlace channel={channel} />
            )}
          </span>
        </CallPlaceLink>
        {/* Someone who may not speak here listens only, and has no microphone to mute. */}
        {call.canSpeak || call.status !== "connected" ? (
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
        ) : (
          <Tooltip text={m.voice.listeningOnly}>
            <span aria-label={m.voice.listeningOnly} className={buttonClass + " text-ink-faint"}>
              <MicrophoneSlashIcon size={18} aria-hidden="true" />
            </span>
          </Tooltip>
        )}
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
        {call.status === "connected" && call.canCamera && <CameraToggle />}
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
      {call.status === "connected" && call.stateRefused !== null && (
        <CallProblem
          text={
            call.stateRefused.retryAfterSeconds === null
              ? m.voice.stateRefused
              : format(m.voice.stateRefusedWait, {
                  wait: seconds.format(call.stateRefused.retryAfterSeconds),
                })
          }
          onDismiss={() => {
            sync.voice.clearStateRefusal();
          }}
        />
      )}
      {call.status === "connected" && call.cameraError !== null && (
        <CameraProblem failure={call.cameraError} />
      )}
    </div>
  );
}

/** Where the call is: a voice channel's name, or a DM's people. */
function CallPlace({ channel }: { channel: Channel }) {
  return isDm(channel) ? <DmTitle channel={channel} /> : channel.name;
}

function DmTitle({ channel }: { channel: Channel }) {
  return useDmTitle(channel);
}

/**
 * The call bar's status and place, as a link to the call's room (the voice channel's screen,
 * or the DM), once the channel is known.
 */
function CallPlaceLink({
  channel,
  children,
}: {
  channel: Channel | undefined;
  children: ReactNode;
}) {
  const domain = useDomain();
  const className = "flex min-w-0 flex-1 flex-col";
  if (channel === undefined) {
    return <span className={className}>{children}</span>;
  }
  return (
    <Link
      {...channelLink({ domain, community: channel.community ?? null }, channel.id)}
      className={
        className +
        " rounded-md outline-none hover:[&>span:last-child]:underline focus-visible:ring-2 focus-visible:ring-accent/50"
      }
    >
      {children}
    </Link>
  );
}

/**
 * Turns the camera on and off. While the camera's last try failed it shows in the danger
 * colour, and `CameraProblem` under the bar says why.
 */
function CameraToggle() {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const on = call.localCamera !== null;
  const label = on ? m.voice.cameraOff : m.voice.cameraOn;
  return (
    <Tooltip text={label}>
      <Button
        aria-label={label}
        aria-pressed={on}
        onPress={() => {
          if (on) {
            sync.voice.stopCamera();
          } else {
            sync.voice.clearCameraError();
            // A failure is shown from the call's state, as `CameraProblem`.
            void sync.voice.startCamera().catch(() => undefined);
          }
        }}
        className={
          buttonClass + (on ? " text-accent" : call.cameraError !== null ? " text-danger" : "")
        }
      >
        {on ? (
          <VideoCameraIcon size={18} aria-hidden="true" />
        ) : (
          <VideoCameraSlashIcon size={18} aria-hidden="true" />
        )}
      </Button>
    </Tooltip>
  );
}

/** Why the camera did not turn on, and what to do about it, until dismissed or tried again. */
function CameraProblem({ failure }: { failure: CameraFailure }) {
  const m = useMessages();
  const sync = useSync();
  const text = {
    none: m.voice.cameraNone,
    denied: m.voice.cameraDenied,
    failed: m.voice.cameraFailed,
    unsent: m.voice.cameraUnsent,
  }[failure];
  return (
    <CallProblem
      text={text}
      onDismiss={() => {
        sync.voice.clearCameraError();
      }}
    />
  );
}

/** Something in the call that did not work, under the bar, until dismissed. */
function CallProblem({ text, onDismiss }: { text: string; onDismiss: () => void }) {
  const m = useMessages();
  return (
    <div className="mt-2 flex items-start gap-2 rounded-md bg-danger-soft px-2 py-1.5 text-xs text-danger">
      <WarningCircleIcon size={16} aria-hidden="true" className="mt-px shrink-0" />
      <p role="alert" className="min-w-0 flex-1">
        {text}
      </p>
      <Tooltip text={m.files.dismiss}>
        <Button
          aria-label={m.files.dismiss}
          onPress={onDismiss}
          className="shrink-0 rounded outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <XIcon size={14} aria-hidden="true" />
        </Button>
      </Tooltip>
    </div>
  );
}
