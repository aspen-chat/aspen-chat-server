import type { Channel } from "@aspen/protocol";
import { userMuted } from "@aspen/protocol";
import {
  HeadphonesIcon,
  MicrophoneSlashIcon,
  MonitorIcon,
  PhoneDisconnectIcon,
  PhoneIcon,
  ProhibitIcon,
  SpeakerHighIcon,
  SpeakerSlashIcon,
} from "@phosphor-icons/react";
import { useRef, useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import {
  useBlocked,
  useSilenced,
  useChannelCan,
  useChannelVoice,
  useMe,
  usePreference,
  useSync,
  useUser,
  useVoiceCall,
} from "@/api/hooks";
import { primaryButtonClass } from "@/features/auth/styles";
import { ChannelHeader } from "@/features/channels/ChannelHeader";
import { Avatar } from "@/features/communities/Avatar";
import { displayNameOf } from "@/features/users/profile";
import { ParticipantMenu, ParticipantMenuButton } from "@/features/voice/ParticipantMenu";
import { CallBar } from "@/features/voice/CallBar";
import { ShareControl } from "@/features/voice/ShareControl";
import { Identity } from "@/features/voice/VoiceParticipants";
import { ScreenTile } from "@/features/voice/ScreenTile";
import { FilesPanel } from "@/features/voice/FilesPanel";
import { TransferLinks } from "@/features/voice/TransferLinks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useNow } from "@/features/layout/useNow";
import { dangerButtonClass } from "@/features/invites/dialog";

/**
 * A voice channel's screen: its header, with the share control while the user is in its call,
 * above the call itself (`CallStage`).
 */
export function VoiceScreen({ channel, communityId }: { channel: Channel; communityId: string }) {
  const m = useMessages();
  const call = useVoiceCall();
  const inThisCall = call.status === "connected" && call.channelId === channel.id;
  const [shareError, setShareError] = useState<string | null>(null);
  return (
    <main className="flex min-h-0 flex-1 flex-col">
      <ChannelHeader
        communityId={communityId}
        glyph={<SpeakerHighIcon size={16} aria-hidden="true" />}
        name={channel.name}
      >
        {/* A narrow screen shares from the call bar below instead. */}
        {inThisCall && (
          <div className="hidden md:flex">
            <ShareControl variant="panel" onError={setShareError} />
          </div>
        )}
      </ChannelHeader>
      {shareError !== null && (
        <p role="alert" className="bg-danger-soft px-4 py-2 text-sm text-danger">
          {shareError}
        </p>
      )}
      <CallStage
        channel={channel}
        joinLabel={format(m.voice.join, { channel: channel.name })}
        className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4"
      />
    </main>
  );
}

/**
 * A channel's call, in a voice channel or a DM: the shared screens, one of them large and the
 * rest as thumbnails to pick from, then everyone in the call as tiles, with the transfers under
 * way between them drawn over the tiles, those a DM's call is ringing as darkened tiles, the
 * call's files, and the way in, labelled `joinLabel`. While the user is in the call, or joining
 * it, Leave Call at the top left leaves it, across from `toolbarEnd`. The screens and file offers of people the user blocked, here or on another
 * deployment, are not shown. `className` lays out the scrolling area that holds it all.
 */
export function CallStage({
  channel,
  joinLabel,
  className,
  toolbarEnd,
}: {
  channel: Channel;
  joinLabel: string;
  className: string;
  /** Controls for the toolbar's other end, across from Leave Call. */
  toolbarEnd?: ReactNode;
}) {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const voice = useChannelVoice(channel.id);
  const inThisCall = call.status === "connected" && call.channelId === channel.id;
  const mayJoin = useChannelCan(channel.id, "joinVoice");
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const tiles = useRef<HTMLDivElement>(null);
  // A ring ends at its `until` by the clock, so the clock is read while any ring is out.
  const now = useNow(1000, voice.rings.length > 0);
  const ringing = voice.rings.filter(
    (ring) =>
      Date.parse(ring.until) > now &&
      !voice.participants.some((participant) => participant.user === ring.user),
  );
  const silenced = useSilenced(
    call.status === "connected"
      ? [...call.screens.map((s) => s.user), ...call.cameras.map((c) => c.user)]
      : [],
  );
  const me = useMe()?.id;
  /** The camera each participant shows, their own being the local track. */
  const cameraOf = (userId: string): MediaStreamTrack | null => {
    if (!inThisCall) {
      return null;
    }
    if (userId === me) {
      return call.localCamera;
    }
    if (silenced.has(userId)) {
      return null;
    }
    return call.cameras.find((camera) => camera.user === userId)?.track ?? null;
  };

  const screens: { id: string; user: string | null; track: MediaStreamTrack }[] = inThisCall
    ? [
        ...(call.localScreen === null
          ? []
          : [{ id: "local", user: null, track: call.localScreen }]),
        ...call.screens
          .filter((screen) => !silenced.has(screen.user))
          .map((screen) => ({
            id: screen.consumerId,
            user: screen.user,
            track: screen.track,
          })),
      ]
    : [];
  const focused = screens.find((screen) => screen.id === focusedId) ?? screens[0];

  // Joining counts: Leave Call also calls off a join under way.
  const here = call.status !== "idle" && call.channelId === channel.id;
  return (
    <>
      <div className={className}>
        {(here || toolbarEnd !== undefined) && (
          <div className="flex items-center justify-between gap-2">
            {here ? (
              <Button
                onPress={() => {
                  sync.voice.leave();
                }}
                className={dangerButtonClass + " flex items-center gap-1.5"}
              >
                <PhoneDisconnectIcon size={18} aria-hidden="true" />
                {m.voice.leaveCallButton}
              </Button>
            ) : (
              <span />
            )}
            {toolbarEnd}
          </div>
        )}
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
            <div ref={tiles} className="relative">
              <ul className="@container grid grid-cols-[repeat(auto-fill,minmax(7rem,1fr))] gap-3">
                {voice.participants.map((participant) => (
                  <ParticipantTile
                    key={participant.user}
                    channelId={channel.id}
                    userId={participant.user}
                    speaking={participant.speaking}
                    muted={participant.muted}
                    deafened={participant.deafened}
                    sharingScreen={participant.sharingScreen}
                    camera={cameraOf(participant.user)}
                  />
                ))}
                {ringing.map((ring) => (
                  <RingingTile key={ring.user} userId={ring.user} />
                ))}
              </ul>
              {inThisCall && <TransferLinks links={call.files.links} container={tiles} />}
            </div>
          )}
        </section>
        {inThisCall && <FilesPanel />}
        {!inThisCall && mayJoin && (
          <div className="flex justify-center">
            <Button
              onPress={() => {
                void sync.voice.join(channel.id).catch(() => undefined);
              }}
              isDisabled={call.status === "joining" || call.status === "rejoining"}
              className={primaryButtonClass + " flex items-center gap-2"}
            >
              <PhoneIcon size={18} aria-hidden="true" />
              {joinLabel}
            </Button>
          </div>
        )}
      </div>
      {/* The call bar lives with the channel list, which a narrow screen does not show
          beside the call; it shows here instead, so joining, a failure to join, and the call
          itself can be seen, muted, and left. */}
      {call.status !== "idle" && call.channelId === channel.id && (
        <div className="md:hidden">
          <CallBar />
        </div>
      )}
    </>
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
      expandable
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
  camera,
}: {
  channelId: string;
  userId: string;
  speaking: boolean;
  muted: boolean;
  deafened: boolean;
  sharingScreen: boolean;
  /** Their camera, which takes the avatar's place and widens the tile to the picture's shape. */
  camera: MediaStreamTrack | null;
}) {
  const m = useMessages();
  const user = useUser(userId);
  const self = useMe()?.id === userId;
  const mutedForMe = usePreference(userMuted(userId));
  const blocked = useBlocked(userId);
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  const tile = useRef<HTMLLIElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const avatar =
    camera === null ? <TileAvatar speaking={speaking} name={name} iconId={user?.icon} /> : null;
  return (
    <li
      ref={tile}
      data-voice-tile={userId}
      data-speaking={speaking ? "true" : undefined}
      onContextMenu={(event) => {
        if (!self) {
          event.preventDefault();
          setMenuOpen(true);
        }
      }}
      className={
        "relative flex flex-col items-center gap-2 rounded-lg bg-surface-raised p-3 " +
        // Twice an avatar tile's width where four columns fit, the whole row where they do not.
        (camera === null ? "" : "col-span-full @min-[33rem]:col-span-4")
      }
    >
      {!self && (
        <>
          <ParticipantMenuButton
            name={name}
            onPress={() => {
              setMenuOpen((open) => !open);
            }}
            className="absolute top-1 end-1"
          />
          <ParticipantMenu
            channelId={channelId}
            userId={userId}
            name={name}
            muted={muted}
            sharing={sharingScreen}
            anchorRef={tile}
            isOpen={menuOpen}
            onOpenChange={setMenuOpen}
          />
        </>
      )}
      {camera !== null && (
        <CameraVideo track={camera} speaking={speaking} name={name} mirrored={self} />
      )}
      <Identity
        user={user}
        name={name}
        avatar={avatar}
        className={"flex-col" + (camera === null ? "" : " w-full")}
        anchorRef={tile}
      />
      <span className="flex gap-1 text-ink-faint">
        {muted && <MicrophoneSlashIcon size={14} aria-label={m.voice.mutedMark} />}
        {deafened && <HeadphonesIcon size={14} aria-label={m.voice.deafenedMark} />}
        {sharingScreen && <MonitorIcon size={14} aria-label={m.voice.sharingMark} />}
        {blocked ? (
          <ProhibitIcon size={14} aria-label={m.blocking.blocked} />
        ) : (
          !self &&
          mutedForMe && (
            <SpeakerSlashIcon
              size={14}
              aria-label={m.voice.mutedForYouMark}
              className="text-danger"
            />
          )
        )}
      </span>
    </li>
  );
}

/**
 * Someone a DM's call is ringing: their tile, darkened until they join or the ring ends. Only
 * the darkening says so on screen; a screen reader hears it.
 */
function RingingTile({ userId }: { userId: string }) {
  const m = useMessages();
  const user = useUser(userId);
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  return (
    <li
      data-ringing={userId}
      className="relative flex flex-col items-center gap-2 rounded-lg bg-surface-raised p-3 brightness-75"
    >
      <Avatar name={name} iconId={user?.icon} size="lg" />
      <span className="max-w-full truncate text-sm">{name}</span>
      <span className="sr-only">{m.voice.ringing}</span>
    </li>
  );
}

function TileAvatar({
  speaking,
  name,
  iconId,
}: {
  speaking: boolean;
  name: string;
  iconId: string | null | undefined;
}) {
  const m = useMessages();
  return (
    <span
      className={
        "rounded-full " +
        (speaking ? "ring-4 ring-online ring-offset-2 ring-offset-surface-raised" : "")
      }
      role={speaking ? "img" : undefined}
      aria-label={speaking ? format(m.voice.speaking, { name }) : undefined}
    >
      <Avatar name={name} iconId={iconId} size="lg" />
    </span>
  );
}

/**
 * A participant's camera, in their tile, where it can be made full screen. The user's own is
 * mirrored, as a mirror shows them; everyone else sees it as the camera does. Its sound is the
 * microphone's, played by the call. A ring shows while they speak.
 */
function CameraVideo({
  track,
  speaking,
  name,
  mirrored,
}: {
  track: MediaStreamTrack;
  speaking: boolean;
  name: string;
  mirrored: boolean;
}) {
  const m = useMessages();
  return (
    <div
      data-camera
      data-speaking={speaking ? "true" : undefined}
      className={
        "w-full rounded-lg " +
        (speaking ? "ring-4 ring-online ring-offset-2 ring-offset-surface-raised" : "")
      }
    >
      <ScreenTile
        track={track}
        label={
          speaking ? format(m.voice.speaking, { name }) : format(m.voice.usersCamera, { name })
        }
        className="aspect-video w-full"
        expandable
        shortcut={false}
        mirrored={mirrored}
        caption={false}
      />
    </div>
  );
}
