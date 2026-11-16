import type { Channel, VideoSource } from "@aspen/protocol";
import { userMuted } from "@aspen/protocol";
import {
  MicrophoneSlashIcon,
  MonitorIcon,
  PhoneDisconnectIcon,
  PhoneIcon,
  PhonePauseIcon,
  ProhibitIcon,
  SpeakerHighIcon,
  SpeakerSlashIcon,
} from "@phosphor-icons/react";
import { useParams } from "@tanstack/react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import {
  useBlocked,
  useSilenced,
  useChannel,
  useChannelCan,
  useChannelVoice,
  useIdWizard,
  useMe,
  usePreference,
  useSync,
  useUser,
  useVoiceCall,
} from "@/api/hooks";
import { primaryButtonClass } from "@/features/auth/styles";
import { HeadphonesSlashIcon } from "@/icons/HeadphonesSlashIcon";
import { ChannelHeader } from "@/features/channels/ChannelHeader";
import { Avatar } from "@/features/communities/Avatar";
import { displayNameOf } from "@/features/users/profile";
import { useNameIn } from "@/features/users/nameIn";
import { ParticipantMenu, ParticipantMenuButton } from "@/features/voice/ParticipantMenu";
import { useContextMenu } from "@/features/layout/contextMenuState";
import { trackFacing } from "@/features/settings/audioDevices";
import { CallBar } from "@/features/voice/CallBar";
import { ShareControl } from "@/features/voice/ShareControl";
import { Identity } from "@/features/voice/VoiceParticipants";
import { ScreenTile } from "@/features/voice/ScreenTile";
import { FilesPanel } from "@/features/voice/FilesPanel";
import { TransferLinks } from "@/features/voice/TransferLinks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useNow } from "@/features/layout/useNow";
import { ringing as stillRinging } from "@/features/voice/ringing";
import { dangerButtonClass } from "@/features/invites/dialog";
import { CopyIdButton } from "@/features/layout/CopyId";
import { isMobilePlatform } from "@/features/layout/platform";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { usePictureInPicture } from "@/features/voice/nativeVideo";

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
        channelId={channel.id}
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
        leaveInBar
      />
    </main>
  );
}

/**
 * A channel's call, in a voice channel or a DM: the shared screens, one of them large and the
 * rest as thumbnails to pick from, then everyone in the call as tiles, with the transfers under
 * way between them drawn over the tiles, those a DM's call is ringing as darkened tiles, the
 * call's files (on a computer; a phone or tablet opens them from the call bar), and the way in,
 * labelled `joinLabel`. While the user is in the call, or joining it, Leave Call at the top left
 * leaves it, across from `toolbarEnd`, except where `leaveInBar` says the call bar does. The
 * screens and file offers of people the user blocked, here or on another deployment, are not
 * shown. `className` lays out the scrolling area that holds it all.
 *
 * Where one pane fills the screen (a phone), the call is a grid that pages rather than a list
 * that scrolls (`PagedCall`): each shared screen is a page of its own, and the people are pages
 * of up to `TILES_PER_PAGE` tiles, swiped between and never scrolled under the finger, so a
 * picture drawn natively behind the page (`nativeVideo.tsx`), which follows the page's layout a
 * frame behind, is still while it is looked at. A camera then fills its tile, as a native camera
 * does at any width, the name drawn over it. While the system shows the call as a picture in
 * picture (Android shows the whole page small then), the stage is the one picture the system
 * shows (`PictureInPictureStage`).
 */
export function CallStage({
  channel,
  joinLabel,
  className,
  toolbarEnd,
  leaveInBar = false,
}: {
  channel: Channel;
  joinLabel: string;
  className: string;
  /** Controls for the toolbar's other end, across from Leave Call. */
  toolbarEnd?: ReactNode;
  /**
   * Whether the call bar shows under the call where only one pane fits (`md`), so Leave Call
   * shows only where it does not.
   */
  leaveInBar?: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const voice = useChannelVoice(channel.id);
  const wizard = useIdWizard();
  const inThisCall = call.status === "connected" && call.channelId === channel.id;
  // The call receives video only while a stage of it shows the tiles.
  useEffect(() => (inThisCall ? sync.voice.showVideo() : undefined), [sync, inThisCall]);
  const mayJoin = useChannelCan(channel.id, "joinVoice");
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const tiles = useRef<HTMLDivElement>(null);
  // A ring ends at its `until` with no event, so the clock is read while any ring is out.
  const now = useNow(1000, voice.rings.length > 0);
  const ringing = voice.rings.filter(
    (ring) =>
      stillRinging(ring, sync.clock, now) &&
      !voice.participants.some((participant) => participant.user === ring.user),
  );
  const silenced = useSilenced(
    call.status === "connected"
      ? [...call.screens.map((s) => s.user), ...call.cameras.map((c) => c.user)]
      : [],
  );
  const me = useMe()?.id;
  const paged = useOnePane();
  const inPictureInPicture = usePictureInPicture();
  /** The camera each participant shows, their own being the local picture. */
  const cameraOf = (userId: string): VideoSource | null => {
    if (!inThisCall) {
      return null;
    }
    if (userId === me) {
      return call.localCamera;
    }
    if (silenced.has(userId)) {
      return null;
    }
    return call.cameras.find((camera) => camera.user === userId)?.video ?? null;
  };

  const screens: Screen[] = inThisCall
    ? [
        ...(call.localScreen === null
          ? []
          : [{ id: "local", user: null, video: call.localScreen }]),
        ...call.screens
          .filter((screen) => !silenced.has(screen.user))
          .map((screen) => ({
            id: screen.id,
            user: screen.user,
            video: screen.video,
          })),
      ]
    : [];
  const focused = screens.find((screen) => screen.id === focusedId) ?? screens[0];
  // The picture the system shows as a picture in picture: the focused screen, else the first
  // camera of someone else; never the user's own.
  const pictureInPicture: PictureInPicture | null =
    focused !== undefined
      ? { kind: "screen", id: focused.id, screen: focused }
      : (() => {
          const camera = voice.participants
            .map((participant) => ({ user: participant.user, video: cameraOf(participant.user) }))
            .find(
              (participant): participant is { user: string; video: VideoSource } =>
                participant.user !== me && participant.video !== null,
            );
          return camera === undefined ? null : { kind: "camera", ...camera };
        })();

  // Joining counts: Leave Call also calls off a join under way.
  const here = call.status !== "idle" && call.channelId === channel.id;
  const copiesId = wizard && voice.session !== null;
  if (inThisCall && inPictureInPicture && pictureInPicture !== null) {
    return <PictureInPictureStage shown={pictureInPicture} />;
  }
  if (paged) {
    return (
      <>
        <div className="flex min-h-0 flex-1 flex-col">
          {(toolbarEnd !== undefined || copiesId) && (
            <div className="flex items-center justify-end gap-2 px-4 pt-4">
              {toolbarEnd}
              {voice.session !== null && <CopyIdButton id={voice.session.id} thing="call" />}
            </div>
          )}
          <PagedCall
            channelId={channel.id}
            screens={screens}
            participants={voice.participants}
            ringing={ringing}
            cameraOf={cameraOf}
            pictureInPicture={pictureInPicture}
            links={inThisCall ? call.files.links : []}
          />
          {!inThisCall && mayJoin && (
            <div className="flex justify-center p-4">
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
        {call.status !== "idle" && call.channelId === channel.id && <CallBar />}
      </>
    );
  }
  return (
    <>
      <div className={className}>
        {(here || toolbarEnd !== undefined || copiesId) && (
          <div
            className={
              "items-center justify-between gap-2 " +
              (leaveInBar && toolbarEnd === undefined && !copiesId ? "hidden md:flex" : "flex")
            }
          >
            {here ? (
              <Button
                onPress={() => {
                  sync.voice.leave();
                }}
                className={
                  dangerButtonClass +
                  " items-center gap-1.5 " +
                  (leaveInBar ? "hidden md:flex" : "flex")
                }
              >
                <PhoneDisconnectIcon size={18} aria-hidden="true" />
                {m.voice.leaveCallButton}
              </Button>
            ) : (
              <span />
            )}
            <span className="flex items-center gap-2">
              {toolbarEnd}
              {voice.session !== null && <CopyIdButton id={voice.session.id} thing="call" />}
            </span>
          </div>
        )}
        {focused !== undefined && (
          <section aria-label={m.voice.screensLabel} className="flex min-h-0 flex-col gap-2">
            <FocusedScreen
              screen={focused}
              pictureInPicture={
                pictureInPicture?.kind === "screen" && pictureInPicture.id === focused.id
              }
            />
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
                    held={participant.held}
                    sharingScreen={participant.sharingScreen}
                    camera={cameraOf(participant.user)}
                    pictureInPicture={
                      pictureInPicture?.kind === "camera" &&
                      pictureInPicture.user === participant.user
                    }
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
        {/* A phone or tablet opens the files from the call bar, leaving the room to the call. */}
        {inThisCall && !isMobilePlatform() && <FilesPanel />}
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

/** A shared screen on the stage: the user's own (`user` `null`) or someone else's. */
interface Screen {
  readonly id: string;
  readonly user: string | null;
  readonly video: VideoSource;
}

/** The picture the system shows as a picture in picture, when it shows the call as one. */
type PictureInPicture =
  | { readonly kind: "screen"; readonly id: string; readonly screen: Screen }
  | { readonly kind: "camera"; readonly user: string; readonly video: VideoSource };

function FocusedScreen({
  screen,
  className = "aspect-video max-h-[60vh] w-full",
  pictureInPicture = false,
}: {
  screen: Screen;
  className?: string;
  pictureInPicture?: boolean;
}) {
  const m = useMessages();
  const user = useUser(screen.user ?? undefined);
  const { communityId } = useParams({ strict: false });
  const name = useNameIn(user, communityId);
  return (
    <ScreenTile
      source={screen.video}
      label={screenLabel(m, screen.user, name)}
      className={className}
      expandable
      pictureInPicture={pictureInPicture}
    />
  );
}

function ScreenThumbnail({ screen, selected }: { screen: Screen; selected: boolean }) {
  const m = useMessages();
  const user = useUser(screen.user ?? undefined);
  const { communityId } = useParams({ strict: false });
  const name = useNameIn(user, communityId);
  return (
    <ScreenTile
      source={screen.video}
      label={screenLabel(m, screen.user, name)}
      className={"h-20 w-36 " + (selected ? "forced-outline ring-2 ring-accent" : "")}
    />
  );
}

/**
 * The stage while the system shows the call as a picture in picture: the one picture it shows,
 * filling the page, which Android shows small then; nothing else, so the picture is all that
 * is seen.
 */
function PictureInPictureStage({ shown }: { shown: PictureInPicture }) {
  const m = useMessages();
  const user = useUser(shown.kind === "screen" ? (shown.screen.user ?? undefined) : shown.user);
  const { communityId } = useParams({ strict: false });
  const name = useNameIn(user, communityId) ?? m.unknownUser;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ScreenTile
        source={shown.kind === "screen" ? shown.screen.video : shown.video}
        label={
          shown.kind === "screen"
            ? screenLabel(m, shown.screen.user, name)
            : format(m.voice.usersCamera, { name })
        }
        className="h-full w-full"
        caption={false}
        cover
        rounded={false}
        pictureInPicture
      />
    </div>
  );
}

/** The most tiles a page of the phone's call holds: a grid of two by two. */
const TILES_PER_PAGE = 4;

/**
 * The call as pages a phone swipes between: each shared screen a page of its own, then the
 * people and those being rung, `TILES_PER_PAGE` to a page. The pages snap, so nothing is ever
 * half shown and nothing moves but between pages; the dots say which page shows, and a screen
 * reader hears the page's number. The transfers under way are drawn over the people's pages.
 */
function PagedCall({
  channelId,
  screens,
  participants,
  ringing,
  cameraOf,
  pictureInPicture,
  links,
}: {
  channelId: string;
  screens: readonly Screen[];
  participants: ReturnType<typeof useChannelVoice>["participants"];
  ringing: ReturnType<typeof useChannelVoice>["rings"];
  cameraOf: (userId: string) => VideoSource | null;
  pictureInPicture: PictureInPicture | null;
  links: Parameters<typeof TransferLinks>[0]["links"];
}) {
  const m = useMessages();
  const pager = useRef<HTMLUListElement>(null);
  const tiles = useRef<HTMLDivElement>(null);
  const [page, setPage] = useState(0);
  const people: ReactNode[] = [
    ...participants.map((participant) => (
      <ParticipantTile
        key={participant.user}
        channelId={channelId}
        userId={participant.user}
        speaking={participant.speaking}
        muted={participant.muted}
        deafened={participant.deafened}
        held={participant.held}
        sharingScreen={participant.sharingScreen}
        camera={cameraOf(participant.user)}
        pictureInPicture={
          pictureInPicture?.kind === "camera" && pictureInPicture.user === participant.user
        }
        fills
      />
    )),
    ...ringing.map((ring) => <RingingTile key={ring.user} userId={ring.user} fills />),
  ];
  const peoplePages: ReactNode[][] = [];
  for (let at = 0; at < people.length; at += TILES_PER_PAGE) {
    peoplePages.push(people.slice(at, at + TILES_PER_PAGE));
  }
  const pages = screens.length + peoplePages.length;
  if (pages === 0) {
    return <p className="py-8 text-center text-ink-muted">{m.voice.nobodyHere}</p>;
  }
  const current = Math.min(page, pages - 1);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ul
        ref={pager}
        aria-label={m.voice.callPages}
        tabIndex={0}
        onScroll={() => {
          const element = pager.current;
          if (element !== null && element.clientWidth > 0) {
            // Counted from the start edge, which a right-to-left scroller reports as negative.
            setPage(Math.round(Math.abs(element.scrollLeft) / element.clientWidth));
          }
        }}
        className="flex min-h-0 flex-1 snap-x snap-mandatory overflow-x-auto overflow-y-hidden [scrollbar-width:none] focus-visible:outline-2 focus-visible:outline-accent"
      >
        {screens.map((screen) => (
          <li key={screen.id} className="flex h-full w-full shrink-0 snap-center items-center p-3">
            <FocusedScreen
              screen={screen}
              className="h-full w-full"
              pictureInPicture={
                pictureInPicture?.kind === "screen" && pictureInPicture.id === screen.id
              }
            />
          </li>
        ))}
        {peoplePages.map((shown, index) => (
          <li key={`people-${String(index)}`} className="h-full w-full shrink-0 snap-center p-3">
            <div ref={index === 0 ? tiles : undefined} className="relative h-full">
              <ul
                aria-label={m.voice.participantsLabel}
                className={
                  "grid h-full gap-3 " +
                  (shown.length === 1
                    ? "grid-cols-1 grid-rows-1"
                    : shown.length === 2
                      ? "grid-cols-1 grid-rows-2"
                      : "grid-cols-2 grid-rows-2")
                }
              >
                {shown}
              </ul>
              {index === 0 && <TransferLinks links={links} container={tiles} />}
            </div>
          </li>
        ))}
      </ul>
      {pages > 1 && (
        <div className="flex justify-center gap-1.5 pb-2">
          <span className="sr-only" role="status">
            {format(m.voice.callPage, { page: current + 1, pages })}
          </span>
          {Array.from({ length: pages }, (_, index) => (
            <span
              key={index}
              aria-hidden="true"
              className={"size-1.5 rounded-full " + (index === current ? "bg-ink" : "bg-ink-faint")}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * Someone in the call. Their camera, when on, takes the avatar's place: on a computer it widens
 * the tile to the picture's own shape; in a cell of a phone's page (`fills`), and wherever the
 * picture is drawn natively (a native picture's tile is its hole, and paints nothing around
 * it), it fills the tile, cropped, with the name and marks over its foot.
 */
function ParticipantTile({
  channelId,
  userId,
  speaking,
  muted,
  deafened,
  held,
  sharingScreen,
  camera,
  pictureInPicture = false,
  fills = false,
}: {
  channelId: string;
  userId: string;
  speaking: boolean;
  muted: boolean;
  deafened: boolean;
  held: boolean;
  sharingScreen: boolean;
  camera: VideoSource | null;
  /** Whether the system's picture in picture shows this camera. */
  pictureInPicture?: boolean;
  /** Whether the tile is a cell of a phone's page, which it fills. */
  fills?: boolean;
}) {
  const m = useMessages();
  const user = useUser(userId);
  const self = useMe()?.id === userId;
  const mutedForMe = usePreference(userMuted(userId));
  const blocked = useBlocked(userId);
  const name = useNameIn(user, useChannel(channelId)?.community) ?? m.unknownUser;
  const tile = useRef<HTMLLIElement>(null);
  const menu = useContextMenu();
  const menuButton = useRef<HTMLButtonElement>(null);
  const avatar =
    camera === null ? <TileAvatar speaking={speaking} name={name} iconId={user?.icon} /> : null;
  const covered = camera !== null && (fills || camera.kind === "native");
  const marks = (
    <span className={"flex gap-1 " + (covered ? "text-white/80" : "text-ink-faint")}>
      {held && <PhonePauseIcon size={14} aria-label={m.voice.heldMark} />}
      {muted && <MicrophoneSlashIcon size={14} aria-label={m.voice.mutedMark} />}
      {deafened && <HeadphonesSlashIcon size={14} aria-label={m.voice.deafenedMark} />}
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
  );
  if (covered) {
    return (
      <li
        ref={tile}
        data-voice-tile={userId}
        data-speaking={speaking ? "true" : undefined}
        onContextMenu={(event) => {
          if (user !== undefined) {
            menu.onContextMenu(event);
          }
        }}
        className={
          "motion-grow relative min-h-0 rounded-lg " +
          (fills ? "" : "col-span-full aspect-video @min-[33rem]:col-span-4")
        }
      >
        <CameraVideo
          source={camera}
          speaking={speaking}
          name={name}
          mirrored={self && trackFacing(camera) !== "back"}
          cover
          pictureInPicture={pictureInPicture}
        />
        <div className="pointer-events-none absolute inset-x-0 bottom-0 flex items-center gap-2 rounded-b-lg bg-black/60 px-2 py-1 text-sm text-white">
          <span className="pointer-events-auto min-w-0">
            <Identity
              user={user}
              name={name}
              avatar={null}
              channelId={channelId}
              className="text-white hover:text-white"
              anchorRef={tile}
            />
          </span>
          {marks}
        </div>
        {!self && (
          <ParticipantMenuButton
            ref={menuButton}
            name={name}
            onPress={menu.toggleFromButton}
            className="absolute top-1 end-1 bg-black/60 text-white"
          />
        )}
        {menu.opened && user !== undefined && (
          <ParticipantMenu
            channelId={channelId}
            user={user}
            name={name}
            muted={muted}
            sharing={sharingScreen}
            menu={menu}
            button={menuButton}
          />
        )}
      </li>
    );
  }
  return (
    <li
      ref={tile}
      data-voice-tile={userId}
      data-speaking={speaking ? "true" : undefined}
      onContextMenu={(event) => {
        if (user !== undefined) {
          menu.onContextMenu(event);
        }
      }}
      className={
        "motion-grow relative flex flex-col items-center gap-2 rounded-lg bg-surface-raised p-3 " +
        (fills ? "min-h-0 justify-center " : "") +
        // Twice an avatar tile's width where four columns fit, the whole row where they do not.
        (camera === null ? "" : "col-span-full @min-[33rem]:col-span-4")
      }
    >
      {!self && (
        <ParticipantMenuButton
          ref={menuButton}
          name={name}
          onPress={menu.toggleFromButton}
          className="absolute top-1 end-1"
        />
      )}
      {menu.opened && user !== undefined && (
        <ParticipantMenu
          channelId={channelId}
          user={user}
          name={name}
          muted={muted}
          sharing={sharingScreen}
          menu={menu}
          button={menuButton}
        />
      )}
      {camera !== null && (
        <CameraVideo
          source={camera}
          speaking={speaking}
          name={name}
          mirrored={self && trackFacing(camera) !== "back"}
          pictureInPicture={pictureInPicture}
        />
      )}
      <Identity
        user={user}
        name={name}
        avatar={avatar}
        channelId={channelId}
        className={"flex-col" + (camera === null ? "" : " w-full")}
        anchorRef={tile}
      />
      {marks}
    </li>
  );
}

/**
 * Someone a DM's call is ringing: their tile, darkened until they join or the ring ends. Only
 * the darkening says so on screen; a screen reader hears it.
 */
function RingingTile({ userId, fills = false }: { userId: string; fills?: boolean }) {
  const m = useMessages();
  const user = useUser(userId);
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  return (
    <li
      data-ringing={userId}
      className={
        "motion-grow relative flex flex-col items-center gap-2 rounded-lg bg-surface-raised p-3 brightness-75" +
        (fills ? " min-h-0 justify-center" : "")
      }
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
        "rounded-full transition-shadow " +
        (speaking
          ? "forced-outline ring-4 ring-online ring-offset-2 ring-offset-surface-raised"
          : "")
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
 * mirrored while it faces them, as a mirror shows them, and a phone's back camera is not, since
 * it shows what is in front of them; everyone else sees it as the camera does. Its sound is the
 * microphone's, played by the call. A ring shows while they speak. It takes the camera's own
 * shape, so a phone held upright shows tall and a webcam wide, and tiles are as uneven as that,
 * except where it fills its tile (`cover`), cropped to the tile's shape.
 */
function CameraVideo({
  source,
  speaking,
  name,
  mirrored,
  cover = false,
  pictureInPicture = false,
}: {
  source: VideoSource;
  speaking: boolean;
  name: string;
  mirrored: boolean;
  cover?: boolean;
  pictureInPicture?: boolean;
}) {
  const m = useMessages();
  return (
    <div
      data-camera
      data-speaking={speaking ? "true" : undefined}
      className={cover ? "h-full w-full" : "w-full"}
    >
      <ScreenTile
        source={source}
        label={
          speaking ? format(m.voice.speaking, { name }) : format(m.voice.usersCamera, { name })
        }
        className={
          "transition-shadow " +
          (cover ? "h-full w-full " : "") +
          (speaking ? "forced-outline ring-4 ring-online ring-offset-2 ring-offset-surface" : "")
        }
        natural={!cover}
        cover={cover}
        expandable
        shortcut={false}
        mirrored={mirrored}
        caption={false}
        pictureInPicture={pictureInPicture}
      />
    </div>
  );
}
