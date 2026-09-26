import { HeadphonesIcon, MicrophoneSlashIcon, MonitorIcon } from "@phosphor-icons/react";
import { useChannelVoice, useMe, useUser } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { displayNameOf } from "@/features/users/profile";
import { ParticipantMenu } from "@/features/voice/ParticipantMenu";
import { visibleParticipants } from "@/features/voice/voiceList";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Who is in a voice channel's call, under the channel in the sidebar: a small avatar with a
 * green ring while they speak, their name, and marks for muted, deafened, and sharing. Past fifteen
 * people, the fifteen who spoke most recently.
 */
export function VoiceParticipants({ channelId }: { channelId: string }) {
  const m = useMessages();
  const voice = useChannelVoice(channelId);
  if (voice.session === null || voice.participants.length === 0) {
    return null;
  }
  const shown = visibleParticipants(voice.participants);
  return (
    <ul aria-label={m.voice.participantsLabel} className="mb-1 ml-6 flex flex-col gap-0.5">
      {shown.map((participant) => (
        <ParticipantRow
          key={participant.user}
          channelId={channelId}
          userId={participant.user}
          speaking={participant.speaking}
          muted={participant.muted}
          deafened={participant.deafened}
          sharingScreen={participant.sharingScreen}
        />
      ))}
      {voice.participants.length > shown.length && (
        <li className="px-2 text-xs text-ink-faint">
          {format(m.voice.moreParticipants, {
            count: String(voice.participants.length - shown.length),
          })}
        </li>
      )}
    </ul>
  );
}

function ParticipantRow({
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
      data-voice-user={userId}
      data-speaking={speaking ? "true" : undefined}
      className="group/participant flex items-center gap-1.5 rounded-md px-2 py-0.5 text-sm text-ink-muted"
    >
      <span
        className={
          "rounded-full " +
          (speaking ? "ring-2 ring-online ring-offset-1 ring-offset-surface-raised" : "")
        }
        aria-label={speaking ? format(m.voice.speaking, { name }) : undefined}
        role={speaking ? "img" : undefined}
      >
        <Avatar name={name} iconId={user?.icon} size="sm" />
      </span>
      <span className="truncate">{name}</span>
      {muted && (
        <MicrophoneSlashIcon
          size={14}
          aria-label={m.voice.mutedMark}
          className="shrink-0 text-ink-faint"
        />
      )}
      {deafened && (
        <HeadphonesIcon
          size={14}
          aria-label={m.voice.deafenedMark}
          className="shrink-0 text-ink-faint"
        />
      )}
      {sharingScreen && (
        <MonitorIcon size={14} aria-label={m.voice.sharingMark} className="shrink-0 text-accent" />
      )}
      {!self && (
        <ParticipantMenu
          channelId={channelId}
          userId={userId}
          name={name}
          muted={muted}
          className="ml-auto opacity-0 group-hover/participant:opacity-100 focus-visible:opacity-100 data-[pressed]:opacity-100"
        />
      )}
    </li>
  );
}
