import { MAX_USER_VOLUME, streamMuted, streamVolume, userMuted, userVolume } from "@aspen/protocol";
import { DotsThreeVerticalIcon } from "@phosphor-icons/react";
import type { RefObject } from "react";
import {
  Button,
  Dialog,
  Label,
  Menu,
  MenuItem,
  Popover,
  Slider,
  SliderOutput,
} from "react-aria-components";
import { SliderRail } from "@/features/layout/SliderRail";
import { useBlocked, useChannelCan, usePreference, useSync } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const STEP = 5;
const itemClass =
  "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover data-[danger]:text-danger";

/**
 * Everything one can do to another person in a call, opened by right-clicking them or by the
 * dots button beside them: how loud they are to this user alone, silencing them for this user
 * alone (their volume is kept for when they are unmuted), the same for the sound of a screen
 * they share, set apart from their voice, and moderation (server mute or unmute, removal),
 * offered only with Manage calls.
 */
export function ParticipantMenu({
  channelId,
  userId,
  name,
  muted,
  sharing = false,
  anchorRef,
  isOpen,
  onOpenChange,
}: {
  channelId: string;
  userId: string;
  name: string;
  /** Server-muted, as the participant record says. */
  muted: boolean;
  /** Sharing a screen, whose sound then has a volume of its own. */
  sharing?: boolean;
  anchorRef: RefObject<HTMLElement | null>;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const gain = usePreference(userVolume(userId));
  const mutedForMe = usePreference(userMuted(userId));
  const streamGain = usePreference(streamVolume(userId));
  const streamMutedForMe = usePreference(streamMuted(userId));
  const blocked = useBlocked(userId);
  const moderate = useChannelCan(channelId, "manageCalls");
  const label = format(m.voice.participantActions, { name });
  return (
    <Popover
      triggerRef={anchorRef}
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      placement="end"
      className="w-64 rounded-md border border-line bg-surface-raised p-2 shadow-lg"
    >
      <Dialog aria-label={label} className="flex flex-col gap-2 outline-none">
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
        {(!blocked || moderate) && (
          <Menu
            aria-label={label}
            className="border-t border-line pt-1 outline-none"
            onAction={(key) => {
              if (key === "muteForMe") {
                void sync.setUserMuted(userId, !mutedForMe).catch(() => undefined);
              } else if (key === "muteStreamForMe") {
                void sync.setStreamMuted(userId, !streamMutedForMe).catch(() => undefined);
              } else if (key === "serverMute") {
                onOpenChange(false);
                void sync.muteVoiceParticipant(channelId, userId, !muted).catch(() => undefined);
              } else if (key === "kick") {
                onOpenChange(false);
                void sync.kickVoiceParticipant(channelId, userId).catch(() => undefined);
              }
            }}
          >
            {!blocked && (
              <MenuItem id="muteForMe" className={itemClass}>
                {mutedForMe ? m.voice.unmuteForMe : m.voice.muteForMe}
              </MenuItem>
            )}
            {!blocked && sharing && (
              <MenuItem id="muteStreamForMe" className={itemClass}>
                {streamMutedForMe ? m.voice.unmuteStreamForMe : m.voice.muteStreamForMe}
              </MenuItem>
            )}
            {moderate && (
              <>
                <MenuItem id="serverMute" className={itemClass}>
                  {muted ? m.voice.serverUnmute : m.voice.serverMute}
                </MenuItem>
                <MenuItem id="kick" className={itemClass} data-danger="true">
                  {m.voice.removeFromCall}
                </MenuItem>
              </>
            )}
          </Menu>
        )}
      </Dialog>
    </Popover>
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
  className,
}: {
  name: string;
  onPress: () => void;
  className?: string;
}) {
  const m = useMessages();
  return (
    <Button
      aria-label={format(m.voice.participantActions, { name })}
      onPress={onPress}
      className={
        "rounded-md p-0.5 text-ink-faint outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 " +
        (className ?? "")
      }
    >
      <DotsThreeVerticalIcon size={16} aria-hidden="true" />
    </Button>
  );
}
