import type { Channel } from "@aspen/protocol";
import {
  HeadphonesIcon,
  MicrophoneSlashIcon,
  MonitorIcon,
  PhoneIcon,
  ScreencastIcon,
  SpeakerHighIcon,
} from "@phosphor-icons/react";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useChannelVoice, useMe, useSync, useUser, useVoiceCall } from "@/api/hooks";
import { primaryButtonClass } from "@/features/auth/styles";
import { ChannelHeader } from "@/features/channels/ChannelHeader";
import { Avatar } from "@/features/communities/Avatar";
import { displayNameOf } from "@/features/users/profile";
import { ParticipantMenu } from "@/features/voice/ParticipantMenu";
import { ScreenTile } from "@/features/voice/ScreenTile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A voice channel's screen: the shared screens, one of them large and the rest as thumbnails
 * to pick from, then everyone in the call as tiles, and the way in or the share control.
 */
export function VoiceScreen({ channel, communityId }: { channel: Channel; communityId: string }) {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const voice = useChannelVoice(channel.id);
  const inThisCall = call.status === "connected" && call.channelId === channel.id;
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [shareError, setShareError] = useState<string | null>(null);

  const screens: { id: string; user: string | null; track: MediaStreamTrack }[] = inThisCall
    ? [
        ...(call.localScreen === null
          ? []
          : [{ id: "local", user: null, track: call.localScreen }]),
        ...call.screens.map((screen) => ({
          id: screen.consumerId,
          user: screen.user,
          track: screen.track,
        })),
      ]
    : [];
  const focused = screens.find((screen) => screen.id === focusedId) ?? screens[0];

  const share = async () => {
    setShareError(null);
    try {
      await sync.voice.startScreenShare();
    } catch (error) {
      // Declining the browser's picker is an ordinary outcome, not an error to show.
      if (!(error instanceof DOMException && error.name === "NotAllowedError")) {
        setShareError(error instanceof Error ? error.message : String(error));
      }
    }
  };

  return (
    <main className="flex min-h-0 flex-1 flex-col">
      <ChannelHeader
        communityId={communityId}
        glyph={<SpeakerHighIcon size={16} aria-hidden="true" />}
        name={channel.name}
      >
        {inThisCall && (
          <Button
            onPress={() => {
              if (call.sharingScreen) {
                sync.voice.stopScreenShare();
              } else {
                void share();
              }
            }}
            aria-pressed={call.sharingScreen}
            className={
              "flex items-center gap-1.5 rounded-md px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-accent/50 " +
              (call.sharingScreen
                ? "bg-danger-soft text-danger hover:bg-danger-soft/80"
                : "text-ink-muted hover:bg-surface-hover hover:text-ink")
            }
          >
            <ScreencastIcon size={18} aria-hidden="true" />
            {call.sharingScreen ? m.voice.stopSharing : m.voice.shareScreen}
          </Button>
        )}
      </ChannelHeader>
      {shareError !== null && (
        <p role="alert" className="bg-danger-soft px-4 py-2 text-sm text-danger">
          {shareError}
        </p>
      )}
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
        {focused !== undefined && (
          <section aria-label={m.voice.screensLabel} className="flex min-h-0 flex-col gap-2">
            <FocusedScreen screen={focused} />
            {screens.length > 1 && (
              <ul className="flex gap-2 overflow-x-auto">
                {screens.map((screen) => (
                  <li key={screen.id}>
                    <Button
                      onPress={() => {
                        setFocusedId(screen.id);
                      }}
                      aria-pressed={screen.id === focused.id}
                      className="rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-accent/50 pressed:opacity-80"
                    >
                      <ScreenThumbnail screen={screen} selected={screen.id === focused.id} />
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        )}
        <section aria-label={m.voice.participantsLabel}>
          {voice.participants.length === 0 ? (
            <p className="py-8 text-center text-ink-muted">{m.voice.nobodyHere}</p>
          ) : (
            <ul className="grid grid-cols-[repeat(auto-fill,minmax(7rem,1fr))] gap-3">
              {voice.participants.map((participant) => (
                <ParticipantTile
                  key={participant.user}
                  channelId={channel.id}
                  userId={participant.user}
                  speaking={participant.speaking}
                  muted={participant.muted}
                  deafened={participant.deafened}
                  sharingScreen={participant.sharingScreen}
                />
              ))}
            </ul>
          )}
        </section>
        {!inThisCall && (
          <div className="flex justify-center">
            <Button
              onPress={() => {
                void sync.voice.join(channel.id).catch(() => undefined);
              }}
              isDisabled={call.status === "joining" || call.status === "rejoining"}
              className={primaryButtonClass + " flex items-center gap-2"}
            >
              <PhoneIcon size={18} aria-hidden="true" />
              {format(m.voice.join, { channel: channel.name })}
            </Button>
          </div>
        )}
      </div>
    </main>
  );
}

function screenLabel(
  m: ReturnType<typeof useMessages>,
  user: string | null,
  name: string | undefined,
): string {
  return user === null
    ? m.voice.yourScreen
    : format(m.voice.usersScreen, { name: name ?? m.unknownUser });
}

function FocusedScreen({ screen }: { screen: { user: string | null; track: MediaStreamTrack } }) {
  const m = useMessages();
  const user = useUser(screen.user ?? undefined);
  return (
    <ScreenTile
      track={screen.track}
      label={screenLabel(m, screen.user, user === undefined ? undefined : displayNameOf(user))}
      className="aspect-video max-h-[60vh] w-full"
    />
  );
}

function ScreenThumbnail({
  screen,
  selected,
}: {
  screen: { user: string | null; track: MediaStreamTrack };
  selected: boolean;
}) {
  const m = useMessages();
  const user = useUser(screen.user ?? undefined);
  return (
    <ScreenTile
      track={screen.track}
      label={screenLabel(m, screen.user, user === undefined ? undefined : displayNameOf(user))}
      className={"h-20 w-36 " + (selected ? "ring-2 ring-accent" : "")}
    />
  );
}

function ParticipantTile({
  channelId,
  userId,
  speaking,
  muted,
  deafened,
  sharingScreen,
}: {
  channelId: string;
  userId: string;
  speaking: boolean;
  muted: boolean;
  deafened: boolean;
  sharingScreen: boolean;
}) {
  const m = useMessages();
  const user = useUser(userId);
  const self = useMe()?.id === userId;
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  return (
    <li
      data-voice-tile={userId}
      data-speaking={speaking ? "true" : undefined}
      className="relative flex flex-col items-center gap-2 rounded-lg bg-surface-raised p-3"
    >
      {!self && (
        <ParticipantMenu
          channelId={channelId}
          userId={userId}
          name={name}
          muted={muted}
          className="absolute top-1 right-1"
        />
      )}
      <span
        className={
          "rounded-full " +
          (speaking ? "ring-4 ring-online ring-offset-2 ring-offset-surface-raised" : "")
        }
        role={speaking ? "img" : undefined}
        aria-label={speaking ? format(m.voice.speaking, { name }) : undefined}
      >
        <Avatar name={name} iconId={user?.icon} size="lg" />
      </span>
      <span className="max-w-full truncate text-sm">{name}</span>
      <span className="flex gap-1 text-ink-faint">
        {muted && <MicrophoneSlashIcon size={14} aria-label={m.voice.mutedMark} />}
        {deafened && <HeadphonesIcon size={14} aria-label={m.voice.deafenedMark} />}
        {sharingScreen && <MonitorIcon size={14} aria-label={m.voice.sharingMark} />}
      </span>
    </li>
  );
}
