import { DotsThreeVerticalIcon } from "@phosphor-icons/react";
import type { RefObject } from "react";
import {
  Button,
  Dialog,
  Header,
  Menu,
  MenuItem,
  MenuSection,
  Popover,
} from "react-aria-components";
import { useMute, useSync } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format, type Messages } from "@/i18n/messages";

/** The lengths of mute offered, in the menu's order; `null` lasts until the user unmutes. */
const MUTE_DURATIONS: readonly {
  key: keyof Messages["mute"]["durations"];
  seconds: number | null;
}[] = [
  { key: "thirtyMinutes", seconds: 30 * 60 },
  { key: "hour", seconds: 60 * 60 },
  { key: "fiveHours", seconds: 5 * 60 * 60 },
  { key: "eightHours", seconds: 8 * 60 * 60 },
  { key: "day", seconds: 24 * 60 * 60 },
  { key: "week", seconds: 7 * 24 * 60 * 60 },
  { key: "forever", seconds: null },
];

/** When a mute ends: the date and time, which a mute of a week at most needs no year for. */
const UNTIL: Intl.DateTimeFormatOptions = {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
};

const itemClass = "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover";
const headerClass = "px-2 pt-1 pb-0.5 text-xs font-semibold text-ink-faint";

/**
 * What the user can do to a channel or DM, opened by right-clicking its row or by its options
 * button: for a text channel or DM, mute it for a while or until they unmute it, or, while it
 * is muted, see until when and unmute it; with `onAccess`, set who can use it; and with
 * `onRename` and `onDelete`, rename or delete it.
 */
export function ChannelMenu({
  channelId,
  name,
  anchorRef,
  isOpen,
  onOpenChange,
  mutable = true,
  onAccess,
  onRename,
  onDelete,
}: {
  channelId: string;
  name: string;
  anchorRef: RefObject<HTMLElement | null>;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  /** Whether muting is offered; a voice channel has nothing to mute. */
  mutable?: boolean;
  /** Opens the channel's access settings, for those who may manage channels. */
  onAccess?: () => void;
  /** Renaming and deleting it, for those who may manage channels or moderate the server. */
  onRename?: () => void;
  onDelete?: () => void;
}) {
  const m = useMessages();
  const until = useDateFormat(UNTIL);
  const sync = useSync();
  const mute = useMute(channelId);
  const label = format(m.mute.options, { name });
  return (
    <Popover
      triggerRef={anchorRef}
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      placement="end top"
      className="w-56 rounded-md border border-line bg-surface-raised p-1 shadow-lg"
    >
      <Dialog aria-label={label} className="outline-none">
        <Menu
          aria-label={label}
          className="outline-none"
          onAction={(key) => {
            onOpenChange(false);
            if (key === "access") {
              onAccess?.();
              return;
            }
            if (key === "rename") {
              onRename?.();
              return;
            }
            if (key === "delete") {
              onDelete?.();
              return;
            }
            if (key === "unmute") {
              void sync.unmuteChannel(channelId).catch(() => undefined);
              return;
            }
            const duration = MUTE_DURATIONS.find((d) => d.key === key);
            if (duration !== undefined) {
              void sync.muteChannel(channelId, duration.seconds).catch(() => undefined);
            }
          }}
        >
          {!mutable ? null : mute === undefined ? (
            <MenuSection>
              <Header className={headerClass}>{m.mute.heading}</Header>
              {MUTE_DURATIONS.map((d) => (
                <MenuItem key={d.key} id={d.key} className={itemClass}>
                  {m.mute.durations[d.key]}
                </MenuItem>
              ))}
            </MenuSection>
          ) : (
            <MenuSection>
              <Header className={headerClass}>
                {mute.until == null
                  ? m.mute.mutedForGood
                  : format(m.mute.mutedUntil, {
                      time: until.format(new Date(mute.until)),
                    })}
              </Header>
              <MenuItem id="unmute" className={itemClass}>
                {m.mute.unmute}
              </MenuItem>
            </MenuSection>
          )}
          {(onAccess !== undefined || onRename !== undefined || onDelete !== undefined) && (
            <MenuSection>
              {onAccess !== undefined && (
                <MenuItem id="access" className={itemClass}>
                  {m.access.open}
                </MenuItem>
              )}
              {onRename !== undefined && (
                <MenuItem id="rename" className={itemClass}>
                  {m.channelActions.rename}
                </MenuItem>
              )}
              {onDelete !== undefined && (
                <MenuItem id="delete" className={itemClass + " text-danger"}>
                  {m.channelActions.delete}
                </MenuItem>
              )}
            </MenuSection>
          )}
        </Menu>
      </Dialog>
    </Popover>
  );
}

/**
 * The button that opens a row's `ChannelMenu`, for keyboards and touch screens, where there is
 * no right click; a long press is taken by dragging the row. Shown on hover and focus, and
 * always on a touch screen.
 */
export function ChannelMenuButton({
  name,
  isOpen,
  onPress,
  className,
}: {
  name: string;
  isOpen: boolean;
  onPress: () => void;
  className?: string;
}) {
  const m = useMessages();
  const label = format(m.mute.options, { name });
  return (
    <Tooltip text={label}>
      <Button
        aria-label={label}
        onPress={onPress}
        className={
          "tap-target shrink-0 rounded p-0.5 text-ink-faint outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50 " +
          (isOpen
            ? "opacity-100"
            : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100 pointer-coarse:opacity-100") +
          (className === undefined ? "" : " " + className)
        }
      >
        <DotsThreeVerticalIcon size={14} aria-hidden="true" />
      </Button>
    </Tooltip>
  );
}
