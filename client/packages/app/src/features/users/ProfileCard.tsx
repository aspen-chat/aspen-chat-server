import type { User } from "@aspen/protocol";
import {
  ChatCircleIcon,
  FlagIcon,
  PhoneIcon,
  PlusIcon,
  ProhibitIcon,
  XIcon,
} from "@phosphor-icons/react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useEffect, useState, type ReactNode, type RefObject } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Menu,
  MenuItem,
  MenuTrigger,
  Popover,
} from "react-aria-components";
import {
  useBlocked,
  useChannel,
  useDeploymentCan,
  useMe,
  useMemberRoles,
  useRoles,
  useSync,
  useUser,
  useIdWizard,
} from "@/api/hooks";
import { useAssignableRoles } from "@/features/community-settings/roleAssignment";
import { Tooltip } from "@/features/layout/Tooltip";
import { Avatar } from "@/features/communities/Avatar";
import {
  dangerButtonClass,
  planeClass,
  planeSurfaceClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { BotBadge, SystemBadge } from "@/features/users/BotBadge";
import { RoleSwatch } from "@/features/users/RoleSwatch";
import { useNameColor } from "@/features/users/nameColor";
import { displayNameOf, statusLine, handleOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useDomain, channelLink } from "@/features/messages/links";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { CopyIdButton } from "@/features/layout/CopyId";
import { ReportModal } from "@/features/reports/ReportDialog";

/**
 * A user's profile as a card, its parts on planes: who they are, their pronouns, what they are up to, and their
 * bio, with ways to message, block, and report them when they are someone else. Opens from any
 * control that names the user, such as a message author or a member row. Opened within a
 * community it shows the roles they hold there, with a way to give them another for those who
 * may. Inside the reader's one-to-one DM with them it offers no way to message them, which is
 * where the reader already is. A block takes away messaging and calling, except for a holder of
 * Message any user, who reaches anyone. The system account's card offers none of these: it
 * sends notices, and is not messaged, called, blocked, or reported.
 */
export function ProfileCard({ user }: { user: User }) {
  const m = useMessages();
  const me = useMe();
  const blocked = useBlocked(user.id);
  const wizard = useIdWizard();
  const { channelId, communityId } = useParams({ strict: false });
  const open = useChannel(channelId ?? "");
  const inTheirDm = open?.ty === "dm" && open.recipients.includes(user.id);
  const messagesAnyone = useDeploymentCan("messageAnyUser");
  const name = displayNameOf(user);
  const nameColor = useNameColor(user.id, communityId);
  return (
    <div className="flex w-72 flex-col gap-2 p-2">
      <div className={planeClass}>
        <div className="flex items-center gap-3">
          <Avatar name={name} iconId={user.icon} size="lg" />
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <span className="truncate text-base font-semibold" style={{ color: nameColor }}>
                {name}
              </span>
              {user.bot && <BotBadge />}
              {user.system && <SystemBadge />}
            </div>
            <div className="truncate text-sm text-ink-muted">
              {handleOf(user)}
              {user.pronouns != null && <span> · {user.pronouns}</span>}
            </div>
            {blocked && (
              <div className="mt-0.5 flex items-center gap-1 text-xs font-medium text-ink-faint">
                <ProhibitIcon size={12} aria-hidden="true" />
                {m.blocking.blocked}
              </div>
            )}
          </div>
        </div>
        {user.bot && <BotMaker ownerId={user.botOwner ?? null} />}
        {user.status != null && (
          <p className="text-sm break-words" aria-label={m.profile.statusLabel}>
            {statusLine(user.status)}
          </p>
        )}
      </div>
      {user.bio != null && (
        <section className={planeSurfaceClass}>
          <h3 className="text-xs font-semibold tracking-wide text-ink-faint uppercase">
            {m.profile.bio}
          </h3>
          <p className="mt-1 text-sm break-words whitespace-pre-wrap">{user.bio}</p>
        </section>
      )}
      {communityId !== undefined && (
        <CommunityRoles communityId={communityId} userId={user.id} name={name} />
      )}
      {me !== null && me.id !== user.id && !user.system && (
        <>
          {(!blocked || messagesAnyone) && (!inTheirDm || !user.bot) && (
            <div className="flex gap-2">
              {!inTheirDm && <MessageButton userId={user.id} />}
              {!user.bot && !blocked && <CallButton userId={user.id} name={name} />}
            </div>
          )}
          <BlockControl userId={user.id} name={name} blocked={blocked} />
          <ReportProfileButton userId={user.id} name={name} />
        </>
      )}
      {wizard && (
        <div className="flex justify-end">
          <CopyIdButton id={user.id} thing={user.bot ? "bot" : "user"} />
        </div>
      )}
    </div>
  );
}

/**
 * Blocks the user, once the caller has read what that does and confirmed, or lifts a block at
 * once.
 */
function BlockControl({
  userId,
  name,
  blocked,
}: {
  userId: string;
  name: string;
  blocked: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = (action: Promise<void>) => {
    setPending(true);
    setError(null);
    action.then(
      () => {
        setPending(false);
        setConfirming(false);
      },
      (failure: unknown) => {
        setError(failure instanceof Error ? failure.message : String(failure));
        setPending(false);
      },
    );
  };
  return (
    <div className="flex flex-col gap-2">
      {confirming && !blocked && (
        <div className="flex flex-col gap-1">
          <p className="text-sm font-semibold">{format(m.blocking.blockTitle, { name })}</p>
          <p className="text-xs text-ink-muted">{m.blocking.blockExplained}</p>
        </div>
      )}
      {/* Not disabled while pending: a disabled button drops focus out of the card, which
          then no longer closes on Escape. */}
      <Button
        aria-disabled={pending}
        onPress={() => {
          if (pending) {
            return;
          }
          if (blocked) {
            run(sync.unblockUser(userId));
          } else if (confirming) {
            run(sync.blockUser(userId));
          } else {
            setConfirming(true);
          }
        }}
        className={
          (confirming && !blocked ? dangerButtonClass : secondaryButtonClass) +
          " flex items-center justify-center gap-1.5"
        }
      >
        <ProhibitIcon size={16} aria-hidden="true" />
        {blocked ? m.blocking.unblock : m.blocking.block}
      </Button>
      {error !== null && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
    </div>
  );
}

/**
 * The roles a member holds in the community the card was opened in, highest first, read from
 * the server when they are not among the members already known. Those who may change their
 * roles get a button listing the ones they could give, and an × on each they could take away.
 * A card of someone who is not a member shows nothing here.
 */
function CommunityRoles({
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
  const held = useMemberRoles(communityId, userId);
  const assignable = useAssignableRoles(communityId, userId);
  const [member, setMember] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const known = held !== undefined;
  useEffect(() => {
    if (known) {
      return;
    }
    let current = true;
    sync.loadMember(communityId, userId).then(
      (isMember) => {
        if (current) {
          setMember(isMember);
        }
      },
      (failure: unknown) => {
        if (current) {
          setError(failure instanceof Error ? failure.message : String(failure));
        }
      },
    );
    return () => {
      current = false;
    };
  }, [sync, communityId, userId, known]);
  if (!member) {
    return null;
  }
  if (held === undefined && error === null) {
    return (
      <section aria-busy="true" className={planeSurfaceClass + " flex flex-col gap-1"}>
        <h3 className="text-xs font-semibold tracking-wide text-ink-faint uppercase">
          {m.profile.roles}
        </h3>
        <LoadingLabel />
        <div className="flex flex-wrap gap-1">
          <Skeleton className="h-5 w-16 rounded-full" />
          <Skeleton className="h-5 w-20 rounded-full" />
        </div>
      </section>
    );
  }
  const holding = new Set(held);
  // Roles are kept lowest first; a card lists the highest first.
  const shown = [...roles].reverse().filter((role) => !role.everyone && holding.has(role.id));
  const addable = [...assignable].reverse().filter((role) => !holding.has(role.id));
  const removable = new Set(assignable.map((role) => role.id));
  const change = (roleId: string, held: boolean) => {
    setError(null);
    sync.setMemberRole(communityId, userId, roleId, held).catch((failure: unknown) => {
      setError(failure instanceof Error ? failure.message : String(failure));
    });
  };
  return (
    <section
      aria-labelledby={`roles-${userId}`}
      className={planeSurfaceClass + " flex flex-col gap-1"}
    >
      <h3
        id={`roles-${userId}`}
        className="text-xs font-semibold tracking-wide text-ink-faint uppercase"
      >
        {m.profile.roles}
      </h3>
      <ul className="flex flex-wrap items-center gap-1">
        {shown.map((role) => (
          <li
            key={role.id}
            className={
              "flex items-center gap-1 rounded-full border border-line py-0.5 text-xs " +
              (removable.has(role.id) ? "ps-2 pe-0.5" : "px-2")
            }
          >
            <RoleSwatch role={role} />
            {role.name}
            {removable.has(role.id) && (
              <Tooltip text={format(m.profile.removeRole, { role: role.name, name })}>
                <Button
                  aria-label={format(m.profile.removeRole, { role: role.name, name })}
                  onPress={() => {
                    change(role.id, false);
                  }}
                  className="flex h-4 w-4 items-center justify-center rounded-full text-ink-muted outline-none hover:bg-surface-hover hover:text-danger focus-visible:ring-2 focus-visible:ring-accent/50"
                >
                  <XIcon size={10} weight="bold" aria-hidden="true" />
                </Button>
              </Tooltip>
            )}
          </li>
        ))}
        {shown.length === 0 && addable.length === 0 && (
          <li className="text-xs text-ink-muted">{m.profile.noRoles}</li>
        )}
        {addable.length > 0 && (
          <li>
            <MenuTrigger>
              <Tooltip text={format(m.profile.addRole, { name })}>
                <Button
                  aria-label={format(m.profile.addRole, { name })}
                  className="flex h-6 w-6 items-center justify-center rounded-full border border-dashed border-line text-ink-muted outline-none hover:border-accent hover:text-accent focus-visible:ring-2 focus-visible:ring-accent/50"
                >
                  <PlusIcon size={12} aria-hidden="true" />
                </Button>
              </Tooltip>
              <Popover className="w-48 rounded-md border border-line bg-surface-raised p-1 shadow-lg">
                <Menu
                  aria-label={format(m.profile.addRole, { name })}
                  onAction={(key) => {
                    change(String(key), true);
                  }}
                  className="max-h-64 overflow-y-auto outline-none"
                >
                  {addable.map((role) => (
                    <MenuItem
                      key={role.id}
                      id={role.id}
                      className="cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover"
                    >
                      {role.name}
                    </MenuItem>
                  ))}
                </Menu>
              </Popover>
            </MenuTrigger>
          </li>
        )}
      </ul>
      {error !== null && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
    </section>
  );
}

/** Reports the user's profile to the deployment's moderators, through `ReportModal`. */
function ReportProfileButton({ userId, name }: { userId: string; name: string }) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Button
        aria-label={format(m.reports.reportProfileLabel, { name })}
        className={secondaryButtonClass + " flex items-center justify-center gap-1.5"}
      >
        <FlagIcon size={16} aria-hidden="true" />
        {m.reports.reportProfile}
      </Button>
      <ReportModal target={{ kind: "profile", userId, name }} />
    </DialogTrigger>
  );
}

/** Who made a bot, or that its owner is gone. */
function BotMaker({ ownerId }: { ownerId: string | null }) {
  const m = useMessages();
  const owner = useUser(ownerId ?? undefined);
  return (
    <p className="text-xs text-ink-muted">
      {ownerId === null
        ? m.bots.ownerGone
        : format(m.bots.madeBy, {
            name: owner === undefined ? m.unknownUser : displayNameOf(owner),
          })}
    </p>
  );
}

/**
 * Calls the user: opens the caller's DM with them, making it the first time, and joins its call,
 * starting it when no one is in it yet.
 */
function CallButton({ userId, name }: { userId: string; name: string }) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const domain = useDomain();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const label = format(m.profile.call, { name });
  return (
    <div className="flex flex-col gap-1">
      <Tooltip text={label}>
        <Button
          aria-label={label}
          isDisabled={pending}
          onPress={() => {
            setPending(true);
            setError(null);
            sync
              .openDm([userId])
              .then((dm) => {
                void navigate(channelLink({ domain, community: null }, dm.id));
                return sync.voice.join(dm.id);
              })
              .then(
                () => {
                  setPending(false);
                },
                (failure: unknown) => {
                  setError(failure instanceof Error ? failure.message : String(failure));
                  setPending(false);
                },
              );
          }}
          className={secondaryButtonClass + " flex items-center justify-center px-3"}
        >
          <PhoneIcon size={16} aria-hidden="true" />
        </Button>
      </Tooltip>
      {error !== null && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
    </div>
  );
}

/** Opens the caller's DM with the user, making it the first time. */
function MessageButton({ userId }: { userId: string }) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const domain = useDomain();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-1">
      <Button
        isDisabled={pending}
        onPress={() => {
          setPending(true);
          setError(null);
          sync.openDm([userId]).then(
            (dm) => {
              void navigate(channelLink({ domain, community: null }, dm.id));
            },
            (failure: unknown) => {
              setError(failure instanceof Error ? failure.message : String(failure));
              setPending(false);
            },
          );
        }}
        className="flex items-center justify-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-contrast outline-none hover:bg-accent-strong pressed:opacity-80 disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        <ChatCircleIcon size={16} aria-hidden="true" />
        {m.profile.message}
      </Button>
      {error !== null && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
    </div>
  );
}

/** Wraps a trigger so pressing it opens the user's profile card beside it. */
export function ProfilePopover({
  user,
  children,
  placement = "bottom start",
  anchorRef,
}: {
  user: User;
  children: ReactNode;
  /** Where the card opens relative to its anchor; `end` puts it beside a list row. */
  placement?: "bottom start" | "end";
  /** What the card is positioned against, when not the trigger itself: the whole row, say. */
  anchorRef?: RefObject<HTMLElement | null>;
}) {
  const m = useMessages();
  return (
    <DialogTrigger>
      {children}
      <Popover
        placement={placement}
        {...(anchorRef === undefined ? {} : { triggerRef: anchorRef })}
        className="rounded-lg border border-line bg-surface shadow-lg"
      >
        <Dialog
          aria-label={format(m.profile.cardLabel, { name: displayNameOf(user) })}
          className="outline-none"
        >
          <ProfileCard user={user} />
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}
