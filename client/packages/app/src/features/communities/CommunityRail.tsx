import { useNavigate, useParams } from "@tanstack/react-router";
import { useCommunities, useSync } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { DotsSixVerticalIcon, PlusIcon } from "@phosphor-icons/react";
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
  const { communityId: current } = useParams({ strict: false });
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
      className="flex w-16 shrink-0 flex-col items-center gap-2 overflow-y-auto border-r border-line bg-surface-sunken py-3"
    >
      <GridList
        aria-label={m.communitiesLabel}
        items={communities}
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
            aria-label={community.name}
            className={
              "group relative cursor-pointer rounded-full outline-none focus-visible:ring-2 focus-visible:ring-accent/60 dragging:opacity-50 " +
              (community.id === current
                ? "ring-2 ring-accent ring-offset-2 ring-offset-surface-sunken"
                : "")
            }
          >
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
