import type { Community, NotificationLevel } from "@aspen/protocol";
import {
  BellIcon,
  CheckCircleIcon,
  GearIcon,
  LinkIcon,
  SignOutIcon,
  UserPlusIcon,
} from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import {
  Header,
  Menu,
  MenuItem,
  MenuSection,
  Popover,
  SubmenuTrigger,
} from "react-aria-components";
import { useCan, useChannels, useCommunityNotificationLevel, useMe, useSync } from "@/api/hooks";
import { useAspenClient } from "@/api/context";
import { CommunitySettingsModal } from "@/features/community-settings/CommunitySettingsDialog";
import { InviteModal } from "@/features/invites/InviteDialog";
import { copyText } from "@/features/layout/clipboard";
import { ConfirmModal } from "@/features/layout/ConfirmModal";
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
import { communityUrl } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const LEVELS: readonly NotificationLevel[] = ["all", "tags", "nothing"];

/** What an item of a community's menu opens outside it, which closes as the item is chosen. */
type CommunityDialog = "invite" | "settings" | "leave";

/**
 * What the reader can do to a community from its icon on the rail, opened by right-clicking it:
 * mark every channel of it read, while one is unread; choose what its channels without a
 * setting of their own tell them of, as its settings do; invite people, for those who may;
 * open its settings; copy the link to it; and leave it, for anyone but its owner, once they
 * have confirmed. It is drawn inside the community's deployment (`SourceScope`), whose sync
 * and client its items use.
 */
export function CommunityMenu({
  menu,
  community,
  unread,
  current,
}: {
  menu: ContextMenuState;
  community: Community;
  unread: boolean;
  /** Whether it is the community shown, which leaving it leaves for the home screen. */
  current: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const client = useAspenClient();
  const navigate = useNavigate();
  const me = useMe();
  const channels = useChannels(community.id);
  const level = useCommunityNotificationLevel(community.id);
  const createInvites = useCan(community.id, "createInvites");
  const manageInvites = useCan(community.id, "manageInvites");
  const [dialog, setDialog] = useState<CommunityDialog | null>(null);
  const label = format(m.mute.options, { name: community.name });
  const close = () => {
    menu.onOpenChange(false);
  };
  const open = (next: CommunityDialog) => () => {
    setDialog(next);
  };
  const dialogProps = (which: CommunityDialog) => ({
    isOpen: dialog === which,
    onOpenChange: (isOpen: boolean) => {
      if (!isOpen) {
        setDialog(null);
      }
    },
  });
  return (
    <>
      <ContextMenu menu={menu} label={label}>
        <Menu
          aria-label={label}
          autoFocus={menu.byKeyboard ? "first" : false}
          onAction={close}
          className={menuClass}
        >
          {unread && (
            <MenuAction
              label={m.contextMenu.markRead}
              icon={<CheckCircleIcon aria-hidden="true" />}
              onAction={() => {
                sync
                  .markChannelsRead(channels.map((channel) => channel.id))
                  .catch((failure: unknown) => {
                    toast(failure instanceof Error ? failure.message : String(failure));
                  });
              }}
            />
          )}
          <SubmenuTrigger>
            <MenuItem textValue={m.notifications.menu} className={menuParentClass}>
              <span className="flex shrink-0 text-ink-muted [&>svg]:h-3.5 [&>svg]:w-3.5">
                <BellIcon aria-hidden="true" />
              </span>
              <ParentLabel
                detail={
                  level === null
                    ? format(m.notifications.default, { level: m.notifications.levels.tags })
                    : m.notifications.levels[level]
                }
              >
                {m.notifications.menu}
              </ParentLabel>
            </MenuItem>
            <Popover className={menuPopoverClass} placement="end top">
              <Menu
                aria-label={m.notifications.notifyMe}
                className={menuClass}
                selectionMode="single"
                selectedKeys={[level ?? "default"]}
                onAction={(key) => {
                  close();
                  void sync
                    .setCommunityNotifications(
                      community.id,
                      key === "default" ? null : (key as NotificationLevel),
                    )
                    .catch(() => undefined);
                }}
              >
                <MenuSection>
                  <Header className={menuHeaderClass}>{m.notifications.notifyMe}</Header>
                  <MenuItem id="default" className={menuItemClass + " " + menuCheckClass}>
                    {format(m.notifications.default, { level: m.notifications.levels.tags })}
                  </MenuItem>
                  {LEVELS.map((choice) => (
                    <MenuItem
                      key={choice}
                      id={choice}
                      className={menuItemClass + " " + menuCheckClass}
                    >
                      {m.notifications.levels[choice]}
                    </MenuItem>
                  ))}
                </MenuSection>
              </Menu>
            </Popover>
          </SubmenuTrigger>
          {(createInvites || manageInvites) && (
            <MenuAction
              label={m.invitePeople}
              icon={<UserPlusIcon aria-hidden="true" />}
              onAction={open("invite")}
            />
          )}
          <MenuAction
            label={m.communitySettings.open}
            icon={<GearIcon aria-hidden="true" />}
            onAction={open("settings")}
          />
          <MenuAction
            label={m.reports.copyLink}
            icon={<LinkIcon aria-hidden="true" />}
            onAction={() => {
              void copyText(communityUrl(client.baseUrl, community.id), document.body).then(
                (copied) => {
                  if (copied) {
                    toast(m.reports.copiedLink);
                  }
                },
              );
            }}
          />
          {me !== null && community.owner !== me.id && (
            <>
              <MenuRule />
              <MenuAction
                label={m.communitySettings.leave}
                icon={<SignOutIcon aria-hidden="true" className="rtl:-scale-x-100" />}
                danger
                onAction={open("leave")}
              />
            </>
          )}
          <CopyIdMenuItem id={community.id} thing="community" className={menuItemClass} />
        </Menu>
      </ContextMenu>
      {dialog === "invite" && <InviteModal community={community} {...dialogProps("invite")} />}
      {dialog === "settings" && (
        <CommunitySettingsModal community={community} {...dialogProps("settings")} />
      )}
      {dialog === "leave" && (
        <ConfirmModal
          heading={format(m.contextMenu.leaveHeading, { community: community.name })}
          explanation={format(m.communitySettings.leaveConfirm, { community: community.name })}
          action={m.communitySettings.leave}
          run={async () => {
            await sync.leaveCommunity(community.id);
            if (current) {
              void navigate({ to: "/" });
            }
          }}
          {...dialogProps("leave")}
        />
      )}
    </>
  );
}
