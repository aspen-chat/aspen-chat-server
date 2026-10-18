import type { User } from "@aspen/protocol";
import {
  AtIcon,
  ChatCircleIcon,
  CheckIcon,
  CopyIcon,
  FlagIcon,
  GavelIcon,
  IdentificationBadgeIcon,
  PhoneIcon,
  ProhibitIcon,
  ShieldIcon,
  SignOutIcon,
  TagIcon,
  UserCircleIcon,
  XCircleIcon,
} from "@phosphor-icons/react";
import { useParams } from "@tanstack/react-router";
import { useEffect, useState, type ReactNode, type RefObject } from "react";
import {
  Dialog,
  Menu,
  MenuItem,
  MenuSection,
  Modal,
  ModalOverlay,
  Popover,
  SubmenuTrigger,
  type PopoverProps,
} from "react-aria-components";
import {
  useBlocked,
  useChannel,
  useChannelAccess,
  useCommunity,
  useDeploymentCan,
  useMe,
  useMemberRoles,
  useNickname,
  useRoles,
  useSync,
  useAccess,
} from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { UserBanDialog } from "@/features/admin/UserBanDialog";
import { BanDialog } from "@/features/community-settings/BanDialog";
import {
  useAssignableRoles,
  useMemberModeration,
} from "@/features/community-settings/roleAssignment";
import { dialogClass, overlayClass, planesModalClass, planeClass } from "@/features/invites/dialog";
import { copyText } from "@/features/layout/clipboard";
import { ContextMenu } from "@/features/layout/ContextMenu";
import type { ContextMenuState } from "@/features/layout/contextMenuState";
import { CopyIdMenuItem } from "@/features/layout/CopyId";
import { ConfirmModal } from "@/features/layout/ConfirmModal";
import { DialogHeading } from "@/features/layout/DialogHeading";
import {
  MenuAction,
  MenuRule,
  ParentLabel,
  menuClass,
  menuItemClass,
  menuParentClass,
  menuPopoverClass,
} from "@/features/layout/menu";
import { toast } from "@/features/layout/toast";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { canMentionIn, mentionIn } from "@/features/mentions/composerMentions";
import { ReportModal } from "@/features/reports/ReportDialog";
import { NicknameForm } from "@/features/users/Nickname";
import { ProfileCard } from "@/features/users/ProfileCard";
import { useMayClearNickname, useNameIn } from "@/features/users/nameIn";
import { displayNameOf, handleOf } from "@/features/users/profile";
import { useReach } from "@/features/users/reach";
import { RoleSwatch } from "@/features/users/RoleSwatch";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** What an item of a user's menu opens outside the menu, which closes as the item is chosen. */
export type UserDialog =
  | "profile"
  | "nickname"
  | "block"
  | "report"
  | "reportNickname"
  | "remove"
  | "ban"
  | "deploymentBan";

/** Says why something failed, in a toast, as a menu that has closed cannot. */
function sayFailure(failure: unknown) {
  toast(problemText(failure));
}

/**
 * Everything the reader can do to `user` from a menu, as items to go in one, each offered only
 * where it would be allowed, in groups parted by rules:
 *
 * - reaching them: their profile, a DM, a call, a tag in the message box of the channel open
 *   (`channelId`, else the route's channel), and their username;
 * - between the two of them: the reader's own nickname, on their own menu in a community; and
 *   blocking, unblocking, and reporting someone else's profile or nickname;
 * - moderating them in the community (`communityId`): giving and taking roles, clearing their
 *   nickname, removing and banning them; and on the deployment, banning them from it;
 *
 * and, while the ID wizard is on, their id. What opens a dialog goes through `open`. Opened from
 * their profile card (`profile` false), it does not offer the card again.
 */
export function UserMenuItems({
  user,
  communityId,
  channelId,
  profile = true,
  open,
}: {
  user: User;
  communityId: string | null;
  channelId: string | null;
  profile?: boolean;
  open: (dialog: UserDialog) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const reach = useReach();
  const me = useMe();
  const params = useParams({ strict: false });
  const shown = useChannel(params.channelId ?? "");
  const blocked = useBlocked(user.id);
  const messagesAnyone = useDeploymentCan("messageAnyUser");
  const banUsers = useDeploymentCan("banUsers");
  const nickname = useNickname(communityId ?? undefined, user.id);
  const name = useNameIn(user, communityId) ?? displayNameOf(user);
  const access = useAccess(communityId ?? "");
  const mayClearNickname = useMayClearNickname(communityId ?? "", user.id);
  const { mayRemove, mayBan } = useMemberModeration(communityId ?? "", user.id);
  const assignable = useAssignableRoles(communityId ?? "", user.id);
  const held = useMemberRoles(communityId ?? "", user.id);
  const mentionChannel = channelId ?? params.channelId ?? null;
  const mayMention = useChannelAccess(mentionChannel ?? "").has("mentionMembers");
  const self = me !== null && me.id === user.id;
  const other = me !== null && !self && !user.system;
  const inTheirDm = shown?.ty === "dm" && shown.recipients.includes(user.id);
  const reaching = {
    message: other && !inTheirDm && (!blocked || messagesAnyone),
    call: other && !user.bot && !blocked,
    mention:
      !self &&
      !user.system &&
      mentionChannel !== null &&
      mayMention &&
      canMentionIn(mentionChannel),
  };
  const ownNickname =
    self &&
    communityId !== null &&
    (access?.has("changeNickname") === true || nickname !== undefined);
  const moderating = communityId !== null && !self;
  const roles = communityId !== null && held !== undefined && assignable.take.length > 0;
  const clearNickname = moderating && mayClearNickname && nickname !== undefined;
  const banFromDeployment = other && banUsers && user.homeDomain == null;
  const moderation =
    roles || clearNickname || (moderating && (mayRemove || mayBan)) || banFromDeployment;
  return (
    <>
      <MenuSection aria-label={m.userMenu.reach}>
        {profile && (
          <MenuAction
            label={m.userMenu.profile}
            icon={<UserCircleIcon aria-hidden="true" />}
            onAction={() => {
              open("profile");
            }}
          />
        )}
        {reaching.message && (
          <MenuAction
            label={m.profile.message}
            icon={<ChatCircleIcon aria-hidden="true" />}
            onAction={() => {
              reach.message(user.id).catch(sayFailure);
            }}
          />
        )}
        {reaching.call && (
          <MenuAction
            label={m.userMenu.call}
            icon={<PhoneIcon aria-hidden="true" />}
            onAction={() => {
              reach.call(user.id).catch(sayFailure);
            }}
          />
        )}
        {reaching.mention && mentionChannel !== null && (
          <MenuAction
            label={m.userMenu.mention}
            icon={<AtIcon aria-hidden="true" />}
            onAction={() => {
              mentionIn(mentionChannel, { text: `@${user.name}`, token: `<@${user.id}>` });
            }}
          />
        )}
        <MenuAction
          label={m.userMenu.copyUsername}
          icon={<CopyIcon aria-hidden="true" />}
          onAction={() => {
            void copyText(handleOf(user), document.body).then((copied) => {
              if (copied) {
                toast(m.userMenu.copiedUsername);
              }
            });
          }}
        />
      </MenuSection>
      {(ownNickname || other) && (
        <>
          <MenuRule />
          <MenuSection aria-label={m.userMenu.personal}>
            {ownNickname && (
              <MenuAction
                label={m.userMenu.changeNickname}
                icon={<TagIcon aria-hidden="true" />}
                onAction={() => {
                  open("nickname");
                }}
              />
            )}
            {other && (
              <MenuAction
                label={blocked ? m.blocking.unblock : m.blocking.block}
                icon={<ProhibitIcon aria-hidden="true" />}
                onAction={() => {
                  if (blocked) {
                    sync.unblockUser(user.id).catch(sayFailure);
                  } else {
                    open("block");
                  }
                }}
              />
            )}
            {other && (
              <MenuAction
                label={m.reports.reportProfile}
                icon={<FlagIcon aria-hidden="true" />}
                onAction={() => {
                  open("report");
                }}
              />
            )}
            {other && communityId !== null && nickname !== undefined && (
              <MenuAction
                label={m.reports.reportNickname}
                icon={<FlagIcon aria-hidden="true" />}
                onAction={() => {
                  open("reportNickname");
                }}
              />
            )}
          </MenuSection>
        </>
      )}
      {moderation && (
        <>
          <MenuRule />
          <MenuSection aria-label={m.userMenu.moderation}>
            {roles && <RolesSubmenu communityId={communityId} userId={user.id} name={name} />}
            {clearNickname && (
              <MenuAction
                label={m.profile.clearNickname}
                icon={<XCircleIcon aria-hidden="true" />}
                onAction={() => {
                  sync.clearNickname(communityId, user.id).then(() => {
                    toast(m.profile.nicknameCleared);
                  }, sayFailure);
                }}
              />
            )}
            {moderating && mayRemove && (
              <MenuAction
                label={m.userMenu.remove}
                icon={<SignOutIcon aria-hidden="true" className="rtl:-scale-x-100" />}
                danger
                onAction={() => {
                  open("remove");
                }}
              />
            )}
            {moderating && mayBan && (
              <MenuAction
                label={m.userMenu.ban}
                icon={<GavelIcon aria-hidden="true" />}
                danger
                onAction={() => {
                  open("ban");
                }}
              />
            )}
            {banFromDeployment && (
              <MenuAction
                label={m.deployments.ban}
                icon={<ShieldIcon aria-hidden="true" />}
                danger
                onAction={() => {
                  open("deploymentBan");
                }}
              />
            )}
          </MenuSection>
        </>
      )}
      <CopyIdMenuItem id={user.id} thing={user.bot ? "bot" : "user"} className={menuItemClass} />
    </>
  );
}

/**
 * The member's roles, highest first, in a submenu: each ticked while they hold it, and chosen to
 * give or take it, where the reader may (`useAssignableRoles`); a role they cannot change is
 * shown but not offered. It stays open as roles are chosen, so several can be changed at once.
 */
function RolesSubmenu({
  communityId,
  userId,
  name,
}: {
  communityId: string;
  userId: string;
  name: string;
}) {
  const m = useMessages();
  const sync = useSync();
  const roles = useRoles(communityId);
  const held = useMemberRoles(communityId, userId) ?? [];
  const assignable = useAssignableRoles(communityId, userId);
  const onePane = useOnePane();
  const listed = [...roles].reverse().filter((role) => !role.everyone && role.bot == null);
  const changeable = (roleId: string) =>
    held.includes(roleId)
      ? assignable.take.some((role) => role.id === roleId)
      : assignable.give.some((role) => role.id === roleId);
  return (
    <SubmenuTrigger>
      <MenuItem textValue={m.profile.roles} className={menuParentClass}>
        <span className="flex shrink-0 text-ink-muted [&>svg]:h-3.5 [&>svg]:w-3.5">
          <IdentificationBadgeIcon aria-hidden="true" />
        </span>
        <ParentLabel>{m.profile.roles}</ParentLabel>
      </MenuItem>
      <Popover className={menuPopoverClass} placement={onePane ? "bottom end" : "end top"}>
        <Menu
          aria-label={format(m.members.rolesOf, { name })}
          selectionMode="multiple"
          selectedKeys={held}
          disabledKeys={listed.filter((role) => !changeable(role.id)).map((role) => role.id)}
          onSelectionChange={(keys) => {
            const chosen = keys === "all" ? listed.map((role) => role.id) : [...keys].map(String);
            for (const role of listed) {
              const holds = held.includes(role.id);
              if (chosen.includes(role.id) !== holds && changeable(role.id)) {
                sync.setMemberRole(communityId, userId, role.id, !holds).catch(sayFailure);
              }
            }
          }}
          className={menuClass + " max-h-72"}
        >
          {listed.map((role) => (
            <MenuItem key={role.id} id={role.id} textValue={role.name} className={menuItemClass}>
              {({ isSelected }) => (
                <>
                  <RoleSwatch role={role} />
                  <span className="min-w-0 flex-1 truncate">{role.name}</span>
                  <CheckIcon
                    size={14}
                    aria-hidden="true"
                    className={"shrink-0 text-accent " + (isSelected ? "" : "invisible")}
                  />
                </>
              )}
            </MenuItem>
          ))}
        </Menu>
      </Popover>
    </SubmenuTrigger>
  );
}

/**
 * What an item of `user`'s menu opened (`which`), outside the menu, which has closed: their
 * profile card by where the menu opened (`anchorRef`), or a dialog.
 */
export function UserDialogs({
  user,
  communityId,
  which,
  onClose,
  anchorRef,
}: {
  user: User;
  communityId: string | null;
  which: UserDialog | null;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
}) {
  const m = useMessages();
  const name = useNameIn(user, communityId) ?? displayNameOf(user);
  const nickname = useNickname(communityId ?? undefined, user.id);
  const props = (dialog: UserDialog) => ({
    isOpen: which === dialog,
    onOpenChange: (open: boolean) => {
      if (!open) {
        onClose();
      }
    },
  });
  return (
    <>
      {which === "profile" && (
        <Popover
          triggerRef={anchorRef}
          placement="bottom start"
          {...props("profile")}
          className="rounded-lg border border-line bg-surface shadow-lg"
        >
          <Dialog aria-label={format(m.profile.cardLabel, { name })} className="outline-none">
            <ProfileCard user={user} />
          </Dialog>
        </Popover>
      )}
      {which === "nickname" && communityId !== null && (
        <NicknameModal communityId={communityId} {...props("nickname")} />
      )}
      {which === "block" && <BlockModal userId={user.id} name={name} {...props("block")} />}
      {which === "report" && (
        <ReportModal
          target={{ kind: "profile", userId: user.id, name: displayNameOf(user) }}
          {...props("report")}
        />
      )}
      {which === "reportNickname" && communityId !== null && nickname !== undefined && (
        <ReportModal
          target={{ kind: "nickname", communityId, userId: user.id, nickname }}
          {...props("reportNickname")}
        />
      )}
      {which === "remove" && communityId !== null && (
        <RemoveMemberModal
          communityId={communityId}
          userId={user.id}
          name={name}
          {...props("remove")}
        />
      )}
      {which === "ban" && communityId !== null && (
        <BanDialog communityId={communityId} userId={user.id} name={name} {...props("ban")} />
      )}
      {which === "deploymentBan" && (
        <UserBanDialog
          userId={user.id}
          name={name}
          bot={user.bot}
          {...props("deploymentBan")}
          onBanned={() => undefined}
        />
      )}
    </>
  );
}

/**
 * `user`'s menu (`UserMenuItems`) as a context menu opened by `menu`: at the pointer, or beside
 * `button` where one opened it. `header` goes above the menu (a call's volume sliders), and
 * `children` are items of the menu's own before the user's (a call's, a DM's), parted from them
 * by a rule. It is named `label`, by default for the user. What its items open stays once the
 * menu closes.
 */
export function UserMenu({
  menu,
  label,
  user,
  communityId,
  channelId = null,
  profile = true,
  button,
  placement,
  header,
  children,
}: {
  menu: ContextMenuState;
  label?: string;
  user: User;
  communityId: string | null;
  /** The channel the menu was opened in, whose message box Mention writes in. */
  channelId?: string | null;
  /** Whether it offers their profile card, which it does not from the card itself. */
  profile?: boolean;
  button?: RefObject<HTMLElement | null>;
  placement?: PopoverProps["placement"];
  header?: ReactNode;
  children?: ReactNode;
}) {
  const m = useMessages();
  const sync = useSync();
  const [dialog, setDialog] = useState<UserDialog | null>(null);
  const name = useNameIn(user, communityId) ?? displayNameOf(user);
  const known = useMemberRoles(communityId ?? "", user.id) !== undefined;
  // Whether they are a member, and so which roles they hold, is read as the menu opens.
  useEffect(() => {
    if (menu.isOpen && communityId !== null && !known) {
      void sync.loadMember(communityId, user.id).catch(() => undefined);
    }
  }, [menu.isOpen, sync, communityId, user.id, known]);
  const close = () => {
    menu.onOpenChange(false);
  };
  const named = label ?? format(m.userMenu.label, { name });
  return (
    <>
      <ContextMenu
        menu={menu}
        label={named}
        {...(button === undefined ? {} : { button })}
        {...(placement === undefined ? {} : { placement })}
      >
        {header}
        <Menu
          aria-label={named}
          autoFocus={menu.byKeyboard ? "first" : false}
          onAction={close}
          className={menuClass}
        >
          {children !== undefined && (
            <>
              {children}
              <MenuRule />
            </>
          )}
          <UserMenuItems
            user={user}
            communityId={communityId}
            channelId={channelId}
            profile={profile}
            open={(next) => {
              close();
              setDialog(next);
            }}
          />
        </Menu>
      </ContextMenu>
      <UserDialogs
        user={user}
        communityId={communityId}
        which={dialog}
        onClose={() => {
          setDialog(null);
        }}
        anchorRef={menu.anchorRef}
      />
    </>
  );
}

/** The reader's own nickname in the community, in a modal of its own. */
function NicknameModal({
  communityId,
  isOpen,
  onOpenChange,
}: {
  communityId: string;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const me = useMe();
  const community = useCommunity(communityId);
  if (me === null) {
    return null;
  }
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={planesModalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>
            {format(m.profile.nicknameIn, { community: community?.name ?? "" })}
          </DialogHeading>
          <section className={planeClass}>
            <NicknameForm communityId={communityId} me={me} />
          </section>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** Blocks the user once the reader has read what that does. */
function BlockModal({
  userId,
  name,
  ...overlay
}: {
  userId: string;
  name: string;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  return (
    <ConfirmModal
      heading={format(m.blocking.blockTitle, { name })}
      explanation={m.blocking.blockExplained}
      action={m.blocking.block}
      run={() => sync.blockUser(userId)}
      {...overlay}
    />
  );
}

/** Removes the member from the community once the reader has confirmed it. */
function RemoveMemberModal({
  communityId,
  userId,
  name,
  ...overlay
}: {
  communityId: string;
  userId: string;
  name: string;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const community = useCommunity(communityId);
  return (
    <ConfirmModal
      heading={format(m.userMenu.removeHeading, { name })}
      explanation={format(m.members.removeConfirm, { name, community: community?.name ?? "" })}
      action={m.members.remove}
      run={() => sync.removeMember(communityId, userId)}
      {...overlay}
    />
  );
}
