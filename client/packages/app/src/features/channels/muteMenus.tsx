import type { NotificationLevel } from "@aspen/protocol";
import { BellIcon, BellSlashIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import {
  Header,
  Menu,
  MenuItem,
  MenuSection,
  Popover,
  SubmenuTrigger,
} from "react-aria-components";
import { useCategoryMute, useMute, useOwnMute, useSync } from "@/api/hooks";
import { useMuteEnd } from "@/features/channels/muteEnd";
import {
  MenuRule,
  ParentLabel,
  menuCheckClass,
  menuClass,
  menuHeaderClass,
  menuItemClass,
  menuParentClass,
  menuPopoverClass,
} from "@/features/layout/menu";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { useMessages } from "@/i18n/context";
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

const NOTIFICATION_LEVELS: readonly NotificationLevel[] = ["all", "tags", "nothing"];

/** An icon before a submenu's item, drawn as `MenuAction` draws its own. */
function MenuIcon({ children }: { children: ReactNode }) {
  return (
    <span className="flex shrink-0 text-ink-muted [&>svg]:h-3.5 [&>svg]:w-3.5">{children}</span>
  );
}

/**
 * A submenu of a menu: its item, `label` with what is in force (`detail`) and `icon`, and the
 * menu it opens, beside the menu where there is room and over it on a phone.
 */
function Submenu({
  label,
  detail,
  icon,
  menuLabel,
  children,
}: {
  label: string;
  detail?: string;
  icon: ReactNode;
  menuLabel: string;
  children: ReactNode;
}) {
  const onePane = useOnePane();
  return (
    <SubmenuTrigger>
      <MenuItem textValue={label} className={menuParentClass}>
        <MenuIcon>{icon}</MenuIcon>
        <ParentLabel {...(detail === undefined ? {} : { detail })}>{label}</ParentLabel>
      </MenuItem>
      <Popover className={menuPopoverClass} placement={onePane ? "bottom end" : "end top"}>
        <Menu aria-label={menuLabel} className={menuClass}>
          {children}
        </Menu>
      </Popover>
    </SubmenuTrigger>
  );
}

/** The lengths of mute, each muting for that long through `mute`. */
function DurationItems({
  mute,
  close,
}: {
  mute: (seconds: number | null) => Promise<void>;
  close: () => void;
}) {
  const m = useMessages();
  return MUTE_DURATIONS.map((d) => (
    <MenuItem
      key={d.key}
      id={d.key}
      className={menuItemClass}
      onAction={() => {
        close();
        void mute(d.seconds).catch(() => undefined);
      }}
    >
      {m.mute.durations[d.key]}
    </MenuItem>
  ));
}

/** One item of a submenu that does `run` and closes the menu. */
function Choice({
  id,
  label,
  run,
  close,
}: {
  id: string;
  label: string;
  run: () => Promise<void>;
  close: () => void;
}) {
  return (
    <MenuItem
      id={id}
      className={menuItemClass}
      onAction={() => {
        close();
        void run().catch(() => undefined);
      }}
    >
      {label}
    </MenuItem>
  );
}

/**
 * A channel's or DM's Mute submenu, which names the mute in force. Unmuted, it offers the
 * lengths of mute. Muted by its own setting, it offers unmuting: which, while its category is
 * muted too, keeps it unmuted, since removing its own mute would leave it muted with the
 * category. Muted with its category, it offers keeping it unmuted. Kept unmuted inside a muted
 * category, it offers the lengths of mute and following its category again. `close` closes the
 * menu it is in, which a submenu's item does not of itself.
 */
export function ChannelMuteSubmenu({
  channelId,
  categoryId,
  close,
}: {
  channelId: string;
  /** Its category, whose mute it takes without a setting of its own; `null` for none. */
  categoryId: string | null;
  close: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const muteEnd = useMuteEnd();
  const mute = useMute(channelId);
  const own = useOwnMute(channelId);
  const categoryMuted = useCategoryMute(categoryId ?? "") !== undefined;
  const mutes = (seconds: number | null) => sync.muteChannel(channelId, seconds);
  const follow = () => sync.unmuteChannel(channelId);
  const keep = () => sync.keepChannelUnmuted(channelId);
  const kept = mute === undefined && own?.muted === false && categoryMuted;
  // While muted, the item says until when in its place.
  const label =
    mute === undefined
      ? m.mute.heading
      : mute.category === null
        ? muteEnd(mute)
        : format(m.mute.byCategory, { end: muteEnd(mute) });
  return (
    <Submenu
      label={label}
      {...(kept ? { detail: m.mute.keptUnmuted } : {})}
      icon={<BellSlashIcon aria-hidden="true" />}
      menuLabel={m.mute.heading}
    >
      {mute === undefined ? (
        <>
          <DurationItems mute={mutes} close={close} />
          {kept && (
            <>
              <MenuRule />
              <Choice id="follow" label={m.mute.followCategory} run={follow} close={close} />
            </>
          )}
        </>
      ) : mute.category === null ? (
        <Choice
          id="unmute"
          label={m.mute.unmute}
          run={categoryMuted ? keep : follow}
          close={close}
        />
      ) : (
        <Choice id="keep" label={m.mute.keepUnmuted} run={keep} close={close} />
      )}
    </Submenu>
  );
}

/** A category's Mute submenu: the lengths of mute, or, while it is muted, until when and Unmute. */
export function CategoryMuteSubmenu({
  categoryId,
  close,
}: {
  categoryId: string;
  close: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const muteEnd = useMuteEnd();
  const mute = useCategoryMute(categoryId);
  return (
    <Submenu
      label={mute === undefined ? m.mute.heading : muteEnd(mute)}
      icon={<BellSlashIcon aria-hidden="true" />}
      menuLabel={m.mute.heading}
    >
      {mute === undefined ? (
        <DurationItems mute={(seconds) => sync.muteCategory(categoryId, seconds)} close={close} />
      ) : (
        <Choice
          id="unmute"
          label={m.mute.unmute}
          run={() => sync.unmuteCategory(categoryId)}
          close={close}
        />
      )}
    </Submenu>
  );
}

/**
 * A Notifications submenu, naming the level in force: what it would be without a setting of its
 * own (`inherited`, as Default), or one of the levels, the one chosen (`own`) ticked. Choosing
 * sets it through `choose`, `null` for Default.
 */
export function NotificationsSubmenu({
  own,
  inherited,
  choose,
  close,
}: {
  own: NotificationLevel | null;
  inherited: NotificationLevel;
  choose: (level: NotificationLevel | null) => Promise<void>;
  close: () => void;
}) {
  const m = useMessages();
  const onePane = useOnePane();
  return (
    <SubmenuTrigger>
      <MenuItem textValue={m.notifications.menu} className={menuParentClass}>
        <MenuIcon>
          <BellIcon aria-hidden="true" />
        </MenuIcon>
        <ParentLabel detail={m.notifications.levels[own ?? inherited]}>
          {m.notifications.menu}
        </ParentLabel>
      </MenuItem>
      <Popover className={menuPopoverClass} placement={onePane ? "bottom end" : "end top"}>
        <Menu
          aria-label={m.notifications.notifyMe}
          className={menuClass}
          selectionMode="single"
          selectedKeys={[own ?? "default"]}
          onAction={(key) => {
            close();
            void choose(key === "default" ? null : (key as NotificationLevel)).catch(
              () => undefined,
            );
          }}
        >
          <MenuSection>
            <Header className={menuHeaderClass}>{m.notifications.notifyMe}</Header>
            <MenuItem id="default" className={menuItemClass + " " + menuCheckClass}>
              {format(m.notifications.default, { level: m.notifications.levels[inherited] })}
            </MenuItem>
            {NOTIFICATION_LEVELS.map((level) => (
              <MenuItem key={level} id={level} className={menuItemClass + " " + menuCheckClass}>
                {m.notifications.levels[level]}
              </MenuItem>
            ))}
          </MenuSection>
        </Menu>
      </Popover>
    </SubmenuTrigger>
  );
}
