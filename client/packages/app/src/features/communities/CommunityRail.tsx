import { Link, useMatchRoute, useNavigate, useParams } from "@tanstack/react-router";
import { UNREAD_DMS } from "@aspen/protocol";
import { useCommunities, useSync, useUnreadPlaces } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { ChatsTeardropIcon, DotsSixVerticalIcon, PlusIcon } from "@phosphor-icons/react";
import {
  Button,
  DropIndicator,
  GridList,
  GridListItem,
  useDragAndDrop,
} from "react-aria-components";
import { format } from "@/i18n/messages";
import { AddCommunityDialog } from "@/features/communities/AddCommunityDialog";
import { Avatar } from "@/features/communities/Avatar";
import { reorderIds } from "@/features/layout/reorder";
import { Tooltip } from "@/features/layout/Tooltip";

/** The drag type rail rows carry, so nothing else accepts them and they accept nothing else. */
const COMMUNITY_DRAG_TYPE = "application/x-aspen-community";

/**
 * The narrow column of communities the user belongs to, in the order they arranged them.
 * Dragging a community moves it and the new order is saved to the user's memberships: with
 * the pointer by dragging the avatar, with the keyboard through the handle that appears on
 * focus. It is a grid list rather than a list box for that handle, which list boxes cannot
 * carry.
 */
export function CommunityRail() {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const communities = useCommunities();
  const unread = useUnreadPlaces();
  const { communityId: current } = useParams({ strict: false });
  const matchRoute = useMatchRoute();
  const inDms = matchRoute({ to: "/dms", fuzzy: true }) !== false;
  const { dragAndDropHooks } = useDragAndDrop({
    getItems: (keys) => Array.from(keys, (key) => ({ [COMMUNITY_DRAG_TYPE]: String(key) })),
    acceptedDragTypes: [COMMUNITY_DRAG_TYPE],
    onReorder: (event) => {
      const moved = new Set(Array.from(event.keys, String));
      const ordered = reorderIds(
        communities.map((c) => c.id),
        moved,
        { key: String(event.target.key), dropPosition: event.target.dropPosition },
      );
      void sync.reorderCommunities(ordered).catch(() => undefined);
    },
    renderDropIndicator: (target) => (
      <DropIndicator
        target={target}
        className="h-0.5 w-12 rounded-full bg-transparent drop-target:bg-accent"
      />
    ),
  });
  return (
    <nav
      aria-label={m.communitiesLabel}
      className="flex w-16 shrink-0 flex-col items-center gap-2 overflow-y-auto border-r border-line bg-surface-rail py-3"
    >
      {/* The dot sits beside the link rather than in it, so the link's own round background
          covers it; see `UnreadDot`. */}
      <div className="relative isolate">
        {unread.has(UNREAD_DMS) && <UnreadDot />}
        <Tooltip text={m.dms.label}>
          <Link
            to="/dms"
            aria-label={
              unread.has(UNREAD_DMS) ? format(m.unreadLabel, { name: m.dms.label }) : m.dms.label
            }
            aria-current={inDms ? "page" : undefined}
            className={
              "flex h-12 w-12 items-center justify-center rounded-full bg-surface-raised text-ink-muted outline-none hover:text-accent focus-visible:ring-2 focus-visible:ring-accent/60 " +
              (inDms ? "text-accent ring-2 ring-accent ring-offset-2 ring-offset-surface-rail" : "")
            }
          >
            <ChatsTeardropIcon size={22} aria-hidden="true" />
          </Link>
        </Tooltip>
      </div>
      <div aria-hidden="true" className="h-px w-8 bg-line" />
      <GridList
        aria-label={m.communitiesLabel}
        items={communities}
        // The list caches each item's rendering by its data; the ring around the current
        // community comes from the route, so the route is declared as a dependency.
        dependencies={[current, unread]}
        selectionMode="none"
        onAction={(key) => {
          void navigate({ to: "/communities/$communityId", params: { communityId: String(key) } });
        }}
        dragAndDropHooks={dragAndDropHooks}
        className="flex flex-col items-center gap-2 outline-none"
      >
        {(community) => (
          <GridListItem
            id={community.id}
            textValue={community.name}
            aria-label={
              unread.has(community.id)
                ? format(m.unreadLabel, { name: community.name })
                : community.name
            }
            className={
              "group relative isolate cursor-pointer rounded-full outline-none focus-visible:ring-2 focus-visible:ring-accent/60 dragging:opacity-50 " +
              (community.id === current
                ? "ring-2 ring-accent ring-offset-2 ring-offset-surface-rail"
                : "")
            }
          >
            {unread.has(community.id) && <UnreadDot />}
            <Avatar name={community.name} iconId={community.icon} size="lg" />
            {/* The handle keyboard and screen reader users drag with; it shows only on focus. */}
            <Button
              slot="drag"
              aria-label={format(m.dragCommunity, { community: community.name })}
              className="absolute -right-1 -bottom-1 rounded-full border border-line bg-surface-raised p-0.5 text-ink-faint opacity-0 outline-none focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              <DotsSixVerticalIcon size={12} aria-hidden="true" />
            </Button>
          </GridListItem>
        )}
      </GridList>
      <AddCommunityDialog
        trigger={
          <Button
            aria-label={m.addCommunity}
            className="flex h-12 w-12 items-center justify-center rounded-full border border-dashed border-line text-ink-muted outline-none hover:border-accent hover:text-accent pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/60"
          >
            <PlusIcon size={22} aria-hidden="true" />
          </Button>
        }
      />
    </nav>
  );
}

/**
 * The mark beside a rail entry that has something unread: a dot at its left edge, half tucked
 * under the entry's icon. It goes in an element that `isolate`s a stacking context, so its
 * negative z-index puts it beneath the icon without sending it behind the rail itself.
 */
function UnreadDot() {
  return (
    <span
      aria-hidden="true"
      className="absolute top-1/2 -left-1 -z-10 h-2 w-2 -translate-y-1/2 rounded-full bg-ink"
    />
  );
}
