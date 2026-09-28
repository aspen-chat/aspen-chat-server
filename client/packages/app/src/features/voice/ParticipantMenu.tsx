import { MAX_USER_VOLUME, userMuted, userVolume } from "@aspen/protocol";
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
  SliderThumb,
  SliderTrack,
} from "react-aria-components";
import { usePreference, useSync } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const STEP = 5;
const itemClass =
  "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover data-[danger]:text-danger";

/**
 * Everything one can do to another person in a call, opened by right-clicking them or by the
 * dots button beside them: how loud they are to this user alone, silencing them for this user
 * alone (their volume is kept for when they are unmuted), and moderation (server mute or
 * unmute, removal), which the server allows only with Manage calls.
 */
export function ParticipantMenu({
  channelId,
  userId,
  name,
  muted,
  anchorRef,
  isOpen,
  onOpenChange,
}: {
  channelId: string;
  userId: string;
  name: string;
  /** Server-muted, as the participant record says. */
  muted: boolean;
  anchorRef: RefObject<HTMLElement | null>;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const gain = usePreference(userVolume(userId));
  const mutedForMe = usePreference(userMuted(userId));
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
        <Slider
          value={Math.round(gain * 100)}
          minValue={0}
          maxValue={MAX_USER_VOLUME * 100}
          step={STEP}
          isDisabled={mutedForMe}
          onChange={(value) => {
            if (typeof value === "number") {
              void sync.setUserVolume(userId, value / 100).catch(() => undefined);
            }
          }}
          className="flex w-full flex-col gap-1 px-1 disabled:opacity-60"
        >
          <div className="flex items-center justify-between gap-2">
            <Label className="truncate text-sm font-medium">{m.voice.volume}</Label>
            <SliderOutput className="text-sm tabular-nums text-ink-muted">
              {({ state }) => `${String(state.getThumbValue(0))}%`}
            </SliderOutput>
          </div>
          <SliderTrack className="relative h-6 w-full">
            <div className="absolute top-1/2 h-1 w-full -translate-y-1/2 rounded-full bg-line" />
            <SliderThumb className="top-1/2 h-4 w-4 rounded-full border border-line bg-accent outline-none dragging:bg-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50" />
          </SliderTrack>
        </Slider>
        <Menu
          aria-label={label}
          className="border-t border-line pt-1 outline-none"
          onAction={(key) => {
            if (key === "muteForMe") {
              void sync.setUserMuted(userId, !mutedForMe).catch(() => undefined);
            } else if (key === "serverMute") {
              onOpenChange(false);
              void sync.muteVoiceParticipant(channelId, userId, !muted).catch(() => undefined);
            } else if (key === "kick") {
              onOpenChange(false);
              void sync.kickVoiceParticipant(channelId, userId).catch(() => undefined);
            }
          }}
        >
          <MenuItem id="muteForMe" className={itemClass}>
            {mutedForMe ? m.voice.unmuteForMe : m.voice.muteForMe}
          </MenuItem>
          <MenuItem id="serverMute" className={itemClass}>
            {muted ? m.voice.serverUnmute : m.voice.serverMute}
          </MenuItem>
          <MenuItem id="kick" className={itemClass} data-danger="true">
            {m.voice.removeFromCall}
          </MenuItem>
        </Menu>
      </Dialog>
    </Popover>
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
