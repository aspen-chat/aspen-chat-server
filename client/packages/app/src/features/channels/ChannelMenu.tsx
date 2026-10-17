import type { NotificationLevel, User } from "@aspen/protocol";
import {
  BellIcon,
  BellSlashIcon,
  DotsThreeVerticalIcon,
  EnvelopeSimpleIcon,
  LinkIcon,
  LockKeyIcon,
  MicrophoneSlashIcon,
  PencilSimpleIcon,
  PhoneDisconnectIcon,
  PhoneIcon,
  TrashIcon,
} from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import type { ReactNode, RefObject } from "react";
import {
  Button,
  Header,
  Menu,
  MenuItem,
  MenuSection,
  Popover,
  SubmenuTrigger,
} from "react-aria-components";
import {
  useChannel,
  useMute,
  useNotificationLevel,
  useReadToItsEnd,
  useSync,
  useVoiceCall,
} from "@/api/hooks";
import { useAspenClient } from "@/api/context";
import { copyText } from "@/features/layout/clipboard";
import { ContextMenu } from "@/features/layout/ContextMenu";
import type { ContextMenuState } from "@/features/layout/contextMenuState";
import { CopyIdMenuItem } from "@/features/layout/CopyId";
import {
  MenuAction,
  MenuRule,
  ParentLabel,
  menuCheckClass,
  menuClass,
  menuHeaderClass,
  menuItemClass,
  menuParentClass,
  menuPopoverClass,
} from "@/features/layout/menu";
import { toast } from "@/features/layout/toast";
import { Tooltip } from "@/features/layout/Tooltip";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { useMuteEnd } from "@/features/channels/muteEnd";
import { channelLink, channelUrl, useDomain } from "@/features/messages/links";
import { markUnread } from "@/features/messages/markUnread";
import { UserMenu } from "@/features/users/UserMenu";
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

/**
 * What the user can do to a channel or DM, opened by right-clicking its row (`menu`), or by its
 * options button (`button`), beside which it then opens. A voice channel offers joining its
 * call, with the microphone on or muted, or leaving it while the user is in it. A text channel
 * or DM read to its end offers Mark unread, from its newest message by someone else. It has two
 * submenus: Mute, to mute it for a while or until they unmute it, which reads until when while
 * it is muted and then offers unmuting; and Notifications, naming the level in force, to choose
 * what it tells them of. Copy link copies the link to share for it. With `onAccess` the user can
 * set who can use it, and with `onRename` and `onDelete` rename or delete it. A one-to-one DM
 * names the other person (`user`), and its menu is theirs (`UserMenu`) with these items first.
 */
export function ChannelMenu({
  channelId,
  name,
  menu,
  button,
  mutable = true,
  voice = false,
  user,
  onAccess,
  onRename,
  onDelete,
}: {
  channelId: string;
  name: string;
  menu: ContextMenuState;
  button: RefObject<HTMLElement | null>;
  /** Whether muting is offered; a voice channel has nothing to mute. */
  mutable?: boolean;
  /** Whether it is a voice channel, whose call it offers to join. */
  voice?: boolean;
  user?: User | undefined;
  /** Opens the channel's access settings, for those who may manage channels. */
  onAccess?: () => void;
  /** Renaming and deleting it, for those who may manage channels or moderate the server. */
  onRename?: () => void;
  onDelete?: () => void;
}) {
  const m = useMessages();
  const label = format(m.mute.options, { name });
  // Beside the row where there is room for it and its submenus; a phone has none beside a row
  // that spans the screen, and a popover is only shifted along its other axis, so there the
  // menu opens below its row.
  const placement = useOnePane() ? "bottom end" : "end top";
  const items = (
    <ChannelItems
      channelId={channelId}
      close={() => {
        menu.onOpenChange(false);
      }}
      mutable={mutable}
      voice={voice}
      {...(onAccess === undefined ? {} : { onAccess })}
      {...(onRename === undefined ? {} : { onRename })}
      {...(onDelete === undefined ? {} : { onDelete })}
    />
  );
  if (user !== undefined) {
    return (
      <UserMenu
        menu={menu}
        label={label}
        user={user}
        communityId={null}
        channelId={channelId}
        button={button}
        placement={placement}
      >
        {items}
      </UserMenu>
    );
  }
  return (
    <ContextMenu menu={menu} label={label} button={button} placement={placement}>
      <Menu
        aria-label={label}
        autoFocus={menu.byKeyboard ? "first" : false}
        onAction={() => {
          menu.onOpenChange(false);
        }}
        className={menuClass}
      >
        {items}
      </Menu>
    </ContextMenu>
  );
}

/**
 * `ChannelMenu`'s own items, inside whichever menu holds them; `close` closes it as a submenu's
 * item is chosen, which only the menu's own items do of themselves.
 */
function ChannelItems({
  channelId,
  close,
  mutable,
  voice,
  onAccess,
  onRename,
  onDelete,
}: {
  channelId: string;
  close: () => void;
  mutable: boolean;
  voice: boolean;
  onAccess?: () => void;
  onRename?: () => void;
  onDelete?: () => void;
}) {
  const m = useMessages();
  const muteEnd = useMuteEnd();
  const sync = useSync();
  const client = useAspenClient();
  const navigate = useNavigate();
  const domain = useDomain();
  const mute = useMute(channelId);
  const channel = useChannel(channelId);
  const notify = useNotificationLevel(channelId);
  const call = useVoiceCall();
  // Marking a channel already unread from its newest message would read the rest.
  const markable = useReadToItsEnd(channelId) && mutable;
  const submenuPlacement = useOnePane() ? "bottom end" : "end top";
  const community = channel?.community ?? null;
  const inCall =
    call.channelId === channelId &&
    (call.status === "joining" || call.status === "connected" || call.status === "rejoining");
  const join = (muted: boolean) => {
    void navigate(channelLink({ domain, community }, channelId));
    if (muted) {
      sync.voice.setMuted(true);
    }
    void sync.voice.join(channelId).catch(() => undefined);
  };
  const manages = onAccess !== undefined || onRename !== undefined || onDelete !== undefined;
  return (
    <>
      {voice &&
        (inCall ? (
          <MenuAction
            label={m.voice.leave}
            icon={<PhoneDisconnectIcon aria-hidden="true" />}
            onAction={() => {
              sync.voice.leave();
            }}
          />
        ) : (
          <>
            <MenuAction
              label={m.voice.joinCall}
              icon={<PhoneIcon aria-hidden="true" />}
              onAction={() => {
                join(false);
              }}
            />
            <MenuAction
              label={m.contextMenu.joinMuted}
              icon={<MicrophoneSlashIcon aria-hidden="true" />}
              onAction={() => {
                join(true);
              }}
            />
          </>
        ))}
      {markable && (
        <MenuAction
          label={m.markUnread}
          icon={<EnvelopeSimpleIcon aria-hidden="true" />}
          onAction={() => {
            markUnread(sync, m, channelId);
          }}
        />
      )}
      {mutable && (
        <SubmenuTrigger>
          <MenuItem textValue={m.mute.heading} className={menuParentClass}>
            <MenuIcon>
              <BellSlashIcon aria-hidden="true" />
            </MenuIcon>
            <ParentLabel>{mute === undefined ? m.mute.heading : muteEnd(mute)}</ParentLabel>
          </MenuItem>
          <Popover className={menuPopoverClass} placement={submenuPlacement}>
            <Menu aria-label={m.mute.heading} className={menuClass}>
              {mute === undefined ? (
                MUTE_DURATIONS.map((d) => (
                  <MenuItem
                    key={d.key}
                    id={d.key}
                    className={menuItemClass}
                    onAction={() => {
                      close();
                      void sync.muteChannel(channelId, d.seconds).catch(() => undefined);
                    }}
                  >
                    {m.mute.durations[d.key]}
                  </MenuItem>
                ))
              ) : (
                <MenuItem
                  id="unmute"
                  className={menuItemClass}
                  onAction={() => {
                    close();
                    void sync.unmuteChannel(channelId).catch(() => undefined);
                  }}
                >
                  {m.mute.unmute}
                </MenuItem>
              )}
            </Menu>
          </Popover>
        </SubmenuTrigger>
      )}
      {mutable && (
        <SubmenuTrigger>
          <MenuItem textValue={m.notifications.menu} className={menuParentClass}>
            <MenuIcon>
              <BellIcon aria-hidden="true" />
            </MenuIcon>
            <ParentLabel detail={m.notifications.levels[notify.level]}>
              {m.notifications.menu}
            </ParentLabel>
          </MenuItem>
          <Popover className={menuPopoverClass} placement={submenuPlacement}>
            <Menu
              aria-label={m.notifications.notifyMe}
              className={menuClass}
              selectionMode="single"
              selectedKeys={[notify.own ?? "default"]}
              onAction={(key) => {
                close();
                void sync
                  .setChannelNotifications(
                    channelId,
                    key === "default" ? null : (key as NotificationLevel),
                  )
                  .catch(() => undefined);
              }}
            >
              <MenuSection>
                <Header className={menuHeaderClass}>{m.notifications.notifyMe}</Header>
                <MenuItem id="default" className={menuItemClass + " " + menuCheckClass}>
                  {format(m.notifications.default, {
                    level: m.notifications.levels[notify.inherited],
                  })}
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
      )}
      <MenuAction
        label={m.reports.copyLink}
        icon={<LinkIcon aria-hidden="true" />}
        onAction={() => {
          void copyText(channelUrl(client.baseUrl, community, channelId), document.body).then(
            (copied) => {
              if (copied) {
                toast(m.reports.copiedLink);
              }
            },
          );
        }}
      />
      {manages && <MenuRule />}
      {onAccess !== undefined && (
        <MenuAction
          label={m.access.open}
          icon={<LockKeyIcon aria-hidden="true" />}
          onAction={onAccess}
        />
      )}
      {onRename !== undefined && (
        <MenuAction
          label={m.channelActions.rename}
          icon={<PencilSimpleIcon aria-hidden="true" />}
          onAction={onRename}
        />
      )}
      {onDelete !== undefined && (
        <MenuAction
          label={m.channelActions.delete}
          icon={<TrashIcon aria-hidden="true" />}
          danger
          onAction={onDelete}
        />
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
        className={menuItemClass}
      />
    </>
  );
}

/** An icon before a submenu's item, drawn as `MenuAction` draws its own. */
function MenuIcon({ children }: { children: ReactNode }) {
  return (
    <span className="flex shrink-0 text-ink-muted [&>svg]:h-3.5 [&>svg]:w-3.5">{children}</span>
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
  ref,
  className,
}: {
  name: string;
  isOpen: boolean;
  /** Pressed, from the keyboard (`byKeyboard`) or not. */
  onPress: (byKeyboard: boolean) => void;
  ref?: RefObject<HTMLButtonElement | null>;
  className?: string;
}) {
  const m = useMessages();
  const label = format(m.mute.options, { name });
  return (
    <Tooltip text={label}>
      <Button
        ref={ref}
        aria-label={label}
        onPress={(event) => {
          onPress(event.pointerType === "keyboard");
        }}
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
