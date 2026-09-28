import type { Channel } from "@aspen/protocol";
import { NotePencilIcon, UsersThreeIcon } from "@phosphor-icons/react";
import { Link, Outlet, useNavigate, useParams } from "@tanstack/react-router";
import { Button } from "react-aria-components";
import { useDms, useMe, useSync, useUnread, useUser } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { MAX_DM_PEOPLE } from "@/features/dms/DmHeader";
import { PeoplePicker } from "@/features/dms/PeoplePicker";
import { otherRecipients } from "@/features/dms/dmName";
import { useDmTitle } from "@/features/dms/useDmTitle";
import { Tooltip } from "@/features/layout/Tooltip";
import { unreadMarkClass } from "@/features/channels/ChannelSidebar";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * `/dms`: the caller's DMs and group DMs beside the route's content, the most recently active
 * first. On narrow screens only one of the two is shown, as in a community.
 */
export function DmLayout() {
  const { channelId } = useParams({ strict: false });
  const showing = channelId !== undefined;
  return (
    <>
      <div className={`${showing ? "hidden md:flex" : "flex"} w-full flex-col md:w-64`}>
        <DmSidebar current={channelId} />
      </div>
      <div className={`${showing ? "flex" : "hidden md:flex"} min-w-0 flex-1 flex-col`}>
        <Outlet />
      </div>
    </>
  );
}

function DmSidebar({ current }: { current: string | undefined }) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const dms = useDms();
  return (
    <div className="flex h-full flex-col border-r border-line bg-surface-raised">
      <div className="flex items-center gap-2 border-b border-line px-4 py-2">
        <h1 className="min-w-0 flex-1 truncate font-semibold">{m.dms.label}</h1>
        <PeoplePicker
          trigger={
            <Tooltip text={m.dms.newMessage}>
              <Button
                aria-label={m.dms.newMessage}
                className="tap-target rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
              >
                <NotePencilIcon size={18} aria-hidden="true" />
              </Button>
            </Tooltip>
          }
          heading={m.dms.newMessage}
          confirmLabel={m.dms.start}
          pendingLabel={m.dms.starting}
          exclude={[]}
          max={MAX_DM_PEOPLE - 1}
          onConfirm={async (ids) => {
            const dm = await sync.openDm(ids);
            void navigate({ to: "/dms/$channelId", params: { channelId: dm.id } });
          }}
        />
      </div>
      <nav
        aria-label={m.dms.label}
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-2"
      >
        {dms.map((dm) => (
          <DmRow key={dm.id} dm={dm} current={dm.id === current} />
        ))}
      </nav>
    </div>
  );
}

function DmRow({ dm, current }: { dm: Channel; current: boolean }) {
  const m = useMessages();
  const me = useMe();
  const title = useDmTitle(dm);
  const first = useUser(otherRecipients(dm, me?.id ?? null)[0]);
  const unread = useUnread(dm.id);
  return (
    <Link
      to="/dms/$channelId"
      params={{ channelId: dm.id }}
      aria-current={current ? "page" : undefined}
      className={
        "flex items-center gap-2 rounded-md text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 " +
        (current
          ? "bg-surface-hover px-2 py-1.5 font-medium text-ink"
          : unread
            ? // The border and this padding make up the usual padding, so nothing moves.
              "px-[7px] py-[5px] " + unreadMarkClass
            : "px-2 py-1.5 text-ink-muted")
      }
    >
      {dm.ty === "groupDm" ? (
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-surface-sunken text-ink-muted">
          <UsersThreeIcon size={16} aria-hidden="true" />
        </span>
      ) : (
        <Avatar name={first === undefined ? title : displayNameOf(first)} iconId={first?.icon} />
      )}
      {unread ? (
        <>
          <span aria-hidden="true" className="min-w-0 flex-1 truncate">
            {title}
          </span>
          <span className="sr-only">{format(m.unreadLabel, { name: title })}</span>
        </>
      ) : (
        <span className="min-w-0 flex-1 truncate">{title}</span>
      )}
    </Link>
  );
}

/** `/dms` with nothing chosen: how to start, when there is nothing yet. */
export function DmIndex() {
  const m = useMessages();
  const dms = useDms();
  return (
    <main className="flex flex-1 flex-col items-center justify-center gap-1 p-6 text-center text-ink-muted">
      {dms.length === 0 && <p className="font-medium text-ink">{m.dms.empty}</p>}
      <p className="text-sm">{m.dms.emptyHint}</p>
    </main>
  );
}
