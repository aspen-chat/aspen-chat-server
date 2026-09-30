import type { User } from "@aspen/protocol";
import { userMuted } from "@aspen/protocol";
import {
  HeadphonesIcon,
  MicrophoneSlashIcon,
  MonitorIcon,
  ProhibitIcon,
  SpeakerSlashIcon,
} from "@phosphor-icons/react";
import { useRef, useState, type ReactNode, type RefObject } from "react";
import { Button } from "react-aria-components";
import { useBlocked, useChannelVoice, useMe, usePreference, useUser } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { ProfilePopover } from "@/features/users/ProfileCard";
import { displayNameOf } from "@/features/users/profile";
import { ParticipantMenu, ParticipantMenuButton } from "@/features/voice/ParticipantMenu";
import { visibleParticipants } from "@/features/voice/voiceList";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Who is in a voice channel's call, under the channel in the sidebar: a small avatar with a
 * green ring while they speak, their name, and marks for muted, deafened, sharing, and muted
 * for this user. Past fifteen people, the fifteen who spoke most recently. Clicking a person
 * shows their profile beside them; right-clicking, or the dots, opens what can be done to them.
 */
export function VoiceParticipants({ channelId }: { channelId: string }) {
  const m = useMessages();
  const voice = useChannelVoice(channelId);
  if (voice.session === null || voice.participants.length === 0) {
    return null;
  }
  const shown = visibleParticipants(voice.participants);
  return (
    <ul aria-label={m.voice.participantsLabel} className="mb-1 ms-6 flex flex-col gap-0.5">
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
  const mutedForMe = usePreference(userMuted(userId));
  const blocked = useBlocked(userId);
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  const row = useRef<HTMLLIElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const avatar = (
    <span
      className={
        "rounded-full transition-shadow " +
        (speaking ? "ring-2 ring-online ring-offset-1 ring-offset-surface-raised" : "")
      }
      aria-label={speaking ? format(m.voice.speaking, { name }) : undefined}
      role={speaking ? "img" : undefined}
    >
      <Avatar name={name} iconId={user?.icon} size="sm" />
    </span>
  );
  return (
    <li
      ref={row}
      data-voice-user={userId}
      data-speaking={speaking ? "true" : undefined}
      onContextMenu={(event) => {
        if (!self) {
          event.preventDefault();
          setMenuOpen(true);
        }
      }}
      className="group/participant flex items-center gap-1.5 rounded-md px-2 py-0.5 text-sm text-ink-muted"
    >
      <Identity user={user} name={name} avatar={avatar} anchorRef={row} />
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
      {blocked ? (
        <ProhibitIcon
          size={14}
          aria-label={m.blocking.blocked}
          className="shrink-0 text-ink-faint"
        />
      ) : (
        !self &&
        mutedForMe && (
          <SpeakerSlashIcon
            size={14}
            aria-label={m.voice.mutedForYouMark}
            className="shrink-0 text-danger"
          />
        )
      )}
      {!self && (
        <>
          <ParticipantMenuButton
            name={name}
            onPress={() => {
              setMenuOpen((open) => !open);
            }}
            className="tap-target ms-auto opacity-0 group-hover/participant:opacity-100 pointer-coarse:opacity-100 focus-visible:opacity-100 data-[pressed]:opacity-100"
          />
          <ParticipantMenu
            channelId={channelId}
            userId={userId}
            name={name}
            muted={muted}
            sharing={sharingScreen}
            anchorRef={row}
            isOpen={menuOpen}
            onOpenChange={setMenuOpen}
          />
        </>
      )}
    </li>
  );
}

/** Avatar and name; a click shows the person's profile beside them, and a second click hides it. */
export function Identity({
  user,
  name,
  avatar,
  className,
  anchorRef,
}: {
  user: User | undefined;
  name: string;
  avatar: ReactNode;
  className?: string;
  /** The row or tile the profile card opens beside. */
  anchorRef: RefObject<HTMLElement | null>;
}) {
  const m = useMessages();
  const button = (
    <Button
      aria-label={format(m.profile.show, { name })}
      className={
        "flex min-w-0 items-center gap-1.5 rounded-md text-start outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50 " +
        (className ?? "")
      }
    >
      {avatar}
      <span className="truncate">{name}</span>
    </Button>
  );
  if (user === undefined) {
    return button;
  }
  return (
    <ProfilePopover user={user} placement="end" anchorRef={anchorRef}>
      {button}
    </ProfilePopover>
  );
}
