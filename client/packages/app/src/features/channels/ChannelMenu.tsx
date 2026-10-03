import type { NotificationLevel } from "@aspen/protocol";
import { CaretRightIcon, DotsThreeVerticalIcon } from "@phosphor-icons/react";
import type { ReactNode, RefObject } from "react";
import {
  Button,
  Dialog,
  Header,
  Menu,
  MenuItem,
  MenuSection,
  Popover,
  Separator,
  SubmenuTrigger,
} from "react-aria-components";
import { useChannel, useMute, useNotificationLevel, useSync } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { useMessages } from "@/i18n/context";
import { format, type Messages } from "@/i18n/messages";
import { CopyIdMenuItem } from "@/features/layout/CopyId";
import { useMuteEnd } from "@/features/channels/muteEnd";

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

const popoverClass = "w-56 rounded-md border border-line bg-surface-raised p-1 shadow-lg";
const itemClass = "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover";
/** An item that opens a submenu: its label, what is in force, and an arrow. */
const parentClass = itemClass + " flex items-center gap-2 open:bg-surface-hover";
const headerClass = "px-2 pt-1 pb-0.5 text-xs font-semibold text-ink-faint";
/** A choice among several, marked when it is the one in force. */
const checkClass = "selected:font-medium selected:text-accent";
const NOTIFICATION_LEVELS: readonly NotificationLevel[] = ["all", "tags", "nothing"];

/**
 * What the user can do to a channel or DM, opened by right-clicking its row or by its options
 * button. A text channel or DM has two submenus: Mute, to mute it for a while or until they
 * unmute it, which reads until when while it is muted and then offers unmuting; and
 * Notifications, naming the level in force, to choose what it tells them of. With `onAccess`
 * the user can set who can use it, and with `onRename` and `onDelete` rename or delete it.
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
  const muteEnd = useMuteEnd();
  const sync = useSync();
  const mute = useMute(channelId);
  const channel = useChannel(channelId);
  const notify = useNotificationLevel(channelId);
  const label = format(m.mute.options, { name });
  // Beside the row where there is room for it and its submenus; a phone has none beside a row
  // that spans the screen, and a popover is only shifted along its other axis, so there the
  // menu opens below its row.
  const onePane = useOnePane();
  // A submenu likewise opens over the rest of the menu on a phone rather than off its side.
  const submenuPlacement = onePane ? "bottom end" : "end top";
  return (
    <Popover
      triggerRef={anchorRef}
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      placement={onePane ? "bottom end" : "end top"}
      className={popoverClass}
    >
      <Dialog aria-label={label} className="outline-none">
        <Menu
          aria-label={label}
          className="outline-none"
          onAction={(key) => {
            onOpenChange(false);
            if (key === "access") {
              onAccess?.();
            } else if (key === "rename") {
              onRename?.();
            } else if (key === "delete") {
              onDelete?.();
            }
          }}
        >
          {mutable && (
            <SubmenuTrigger>
              <MenuItem id="mute" className={parentClass}>
                <ParentLabel>{mute === undefined ? m.mute.heading : muteEnd(mute)}</ParentLabel>
              </MenuItem>
              <Popover className={popoverClass} placement={submenuPlacement}>
                <Menu
                  aria-label={m.mute.heading}
                  className="outline-none"
                  onAction={(key) => {
                    onOpenChange(false);
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
                  {mute === undefined ? (
                    MUTE_DURATIONS.map((d) => (
                      <MenuItem key={d.key} id={d.key} className={itemClass}>
                        {m.mute.durations[d.key]}
                      </MenuItem>
                    ))
                  ) : (
                    <MenuItem id="unmute" className={itemClass}>
                      {m.mute.unmute}
                    </MenuItem>
                  )}
                </Menu>
              </Popover>
            </SubmenuTrigger>
          )}
          {mutable && (
            <SubmenuTrigger>
              <MenuItem id="notifications" className={parentClass}>
                <ParentLabel detail={m.notifications.levels[notify.level]}>
                  {m.notifications.menu}
                </ParentLabel>
              </MenuItem>
              <Popover className={popoverClass} placement={submenuPlacement}>
                <Menu
                  aria-label={m.notifications.notifyMe}
                  className="outline-none"
                  selectionMode="single"
                  selectedKeys={[notify.own ?? "default"]}
                  onAction={(key) => {
                    onOpenChange(false);
                    void sync
                      .setChannelNotifications(
                        channelId,
                        key === "default" ? null : (key as NotificationLevel),
                      )
                      .catch(() => undefined);
                  }}
                >
                  <MenuSection>
                    <Header className={headerClass}>{m.notifications.notifyMe}</Header>
                    <MenuItem id="default" className={itemClass + " " + checkClass}>
                      {format(m.notifications.default, {
                        level: m.notifications.levels[notify.inherited],
                      })}
                    </MenuItem>
                    {NOTIFICATION_LEVELS.map((level) => (
                      <MenuItem key={level} id={level} className={itemClass + " " + checkClass}>
                        {m.notifications.levels[level]}
                      </MenuItem>
                    ))}
                  </MenuSection>
                </Menu>
              </Popover>
            </SubmenuTrigger>
          )}
          {mutable &&
            (onAccess !== undefined || onRename !== undefined || onDelete !== undefined) && (
              <Separator className="my-1 h-px bg-line" />
            )}
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
          <CopyIdMenuItem
            id={channelId}
            thing={
              channel?.ty === "dm" || channel?.ty === "groupDm"
                ? "dm"
                : channel?.ty === "thread"
                  ? "thread"
                  : "channel"
            }
            className={itemClass}
          />
        </Menu>
      </Dialog>
    </Popover>
  );
}

/** A submenu's item: its label, what is in force there when given, and an arrow to the submenu. */
function ParentLabel({ children, detail }: { children: ReactNode; detail?: string }) {
  return (
    <>
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {detail !== undefined && <span className="shrink-0 text-xs text-ink-faint">{detail}</span>}
      <CaretRightIcon
        size={12}
        aria-hidden="true"
        className="shrink-0 text-ink-faint rtl:-scale-x-100"
      />
    </>
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
