import {
  MAX_USER_VOLUME,
  streamMuted,
  streamVolume,
  userMuted,
  userVolume,
  type User,
} from "@aspen/protocol";
import {
  DotsThreeVerticalIcon,
  MicrophoneSlashIcon,
  MonitorIcon,
  PhoneDisconnectIcon,
  SpeakerHighIcon,
  SpeakerSlashIcon,
} from "@phosphor-icons/react";
import type { RefObject } from "react";
import { Button, Label, Slider, SliderOutput, type PressEvent } from "react-aria-components";
import type { ContextMenuState } from "@/features/layout/contextMenuState";
import { MenuAction } from "@/features/layout/menu";
import { SliderRail } from "@/features/layout/SliderRail";
import { UserMenu } from "@/features/users/UserMenu";
import {
  useBlocked,
  useChannel,
  useChannelCan,
  useMe,
  usePreference,
  useSync,
  useVoiceMuted,
} from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const STEP = 5;

/**
 * Everything one can do to someone in a call, opened by right-clicking them (`menu`) or by the
 * dots button beside them (`button`): their menu (`UserMenu`), and above it, for someone else,
 * how loud they are to this user alone, then silencing them for this user alone (their volume
 * is kept for when they are unmuted), the same for the sound of a screen they share, set apart
 * from their voice, and moderation (server mute or unmute, removal), offered only with Manage
 * calls.
 */
export function ParticipantMenu({
  channelId,
  user,
  name,
  muted,
  sharing = false,
  menu,
  button,
}: {
  channelId: string;
  user: User;
  name: string;
  /** Server-muted, as the participant record says. */
  muted: boolean;
  /** Sharing a screen, whose sound then has a volume of its own. */
  sharing?: boolean;
  menu: ContextMenuState;
  button: RefObject<HTMLElement | null>;
}) {
  const m = useMessages();
  const self = useMe()?.id === user.id;
  const community = useChannel(channelId)?.community ?? null;
  return (
    <UserMenu
      menu={menu}
      label={format(m.voice.participantActions, { name })}
      user={user}
      communityId={community}
      button={button}
      placement="end"
      {...(self
        ? {}
        : {
            header: <CallVolumes userId={user.id} sharing={sharing} />,
            children: (
              <CallItems channelId={channelId} userId={user.id} muted={muted} sharing={sharing} />
            ),
          })}
    />
  );
}

/**
 * How loud someone else is to this user alone, and the sound of the screen they share apart
 * from their voice; or, while they are blocked, that they are silenced.
 */
function CallVolumes({ userId, sharing }: { userId: string; sharing: boolean }) {
  const m = useMessages();
  const sync = useSync();
  const gain = usePreference(userVolume(userId));
  const mutedForMe = usePreference(userMuted(userId));
  const streamGain = usePreference(streamVolume(userId));
  const streamMutedForMe = usePreference(streamMuted(userId));
  const blocked = useBlocked(userId);
  return (
    <div className="flex flex-col gap-2 border-b border-line px-1 pt-1 pb-2">
      <VolumeSlider
        label={sharing ? m.voice.voiceVolume : m.voice.volume}
        gain={gain}
        isDisabled={mutedForMe || blocked}
        onChange={(value) => {
          void sync.setUserVolume(userId, value).catch(() => undefined);
        }}
      />
      {sharing && !blocked && (
        <VolumeSlider
          label={m.voice.streamVolume}
          gain={streamGain}
          isDisabled={streamMutedForMe}
          onChange={(value) => {
            void sync.setStreamVolume(userId, value).catch(() => undefined);
          }}
        />
      )}
      {blocked && <p className="px-1 text-xs text-ink-muted">{m.blocking.blockedInCall}</p>}
    </div>
  );
}

/** A call's own items of someone else's menu, inside it. */
function CallItems({
  channelId,
  userId,
  muted,
  sharing,
}: {
  channelId: string;
  userId: string;
  muted: boolean;
  sharing: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const mutedForMe = usePreference(userMuted(userId));
  const streamMutedForMe = usePreference(streamMuted(userId));
  const blocked = useBlocked(userId);
  const moderate = useChannelCan(channelId, "manageCalls");
  // The community's mute, which outlasts the call; the participant's record until it is read.
  const serverMuted =
    useVoiceMuted(useChannel(channelId)?.community ?? null, userId, moderate) ?? muted;
  return (
    <>
      {!blocked && (
        <MenuAction
          label={mutedForMe ? m.voice.unmuteForMe : m.voice.muteForMe}
          icon={
            mutedForMe ? (
              <SpeakerHighIcon aria-hidden="true" />
            ) : (
              <SpeakerSlashIcon aria-hidden="true" />
            )
          }
          onAction={() => {
            void sync.setUserMuted(userId, !mutedForMe).catch(() => undefined);
          }}
        />
      )}
      {!blocked && sharing && (
        <MenuAction
          label={streamMutedForMe ? m.voice.unmuteStreamForMe : m.voice.muteStreamForMe}
          icon={<MonitorIcon aria-hidden="true" />}
          onAction={() => {
            void sync.setStreamMuted(userId, !streamMutedForMe).catch(() => undefined);
          }}
        />
      )}
      {moderate && (
        <>
          <MenuAction
            label={serverMuted ? m.voice.serverUnmute : m.voice.serverMute}
            icon={<MicrophoneSlashIcon aria-hidden="true" />}
            onAction={() => {
              void sync
                .muteVoiceParticipant(channelId, userId, !serverMuted)
                .catch(() => undefined);
            }}
          />
          <MenuAction
            label={m.voice.removeFromCall}
            icon={<PhoneDisconnectIcon aria-hidden="true" />}
            danger
            onAction={() => {
              void sync.kickVoiceParticipant(channelId, userId).catch(() => undefined);
            }}
          />
        </>
      )}
    </>
  );
}

/** A volume, 0 to `MAX_USER_VOLUME` as a percentage, in steps of `STEP`. */
function VolumeSlider({
  label,
  gain,
  isDisabled,
  onChange,
}: {
  label: string;
  gain: number;
  isDisabled: boolean;
  onChange: (gain: number) => void;
}) {
  return (
    <Slider
      value={Math.round(gain * 100)}
      minValue={0}
      maxValue={MAX_USER_VOLUME * 100}
      step={STEP}
      isDisabled={isDisabled}
      onChange={(value) => {
        if (typeof value === "number") {
          onChange(value / 100);
        }
      }}
      className="flex w-full flex-col gap-1 px-1 disabled:opacity-60"
    >
      <div className="flex items-center justify-between gap-2">
        <Label className="truncate text-sm font-medium">{label}</Label>
        <SliderOutput className="text-sm tabular-nums text-ink-muted">
          {({ state }) => `${String(state.getThumbValue(0))}%`}
        </SliderOutput>
      </div>
      <SliderRail />
    </Slider>
  );
}

/** The dots button that opens the menu for those who do not right-click. */
export function ParticipantMenuButton({
  name,
  onPress,
  ref,
  className,
}: {
  name: string;
  /** Pressed, from the keyboard (`byKeyboard`) or not. */
  onPress: (byKeyboard: boolean) => void;
  ref?: RefObject<HTMLButtonElement | null>;
  className?: string;
}) {
  const m = useMessages();
  return (
    <Button
      ref={ref}
      aria-label={format(m.voice.participantActions, { name })}
      onPress={(event: PressEvent) => {
        onPress(event.pointerType === "keyboard");
      }}
      className={
        "rounded-md p-0.5 text-ink-faint outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 " +
        (className ?? "")
      }
    >
      <DotsThreeVerticalIcon size={16} aria-hidden="true" />
    </Button>
  );
}
