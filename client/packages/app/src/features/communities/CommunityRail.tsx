import { Link, useMatchRoute, useNavigate, useParams } from "@tanstack/react-router";
import { RAIL_ORDER, UNREAD_DMS, type AspenSync, type Community } from "@aspen/protocol";
import { useState } from "react";
import { SourceScope } from "@/api/deployments";
import { useEverywhere, type Source } from "@/api/everywhere";
import { useIsAdmin, usePreference, useSync } from "@/api/hooks";
import { communityLink } from "@/features/messages/links";
import { arrangeRail, railKey } from "@/features/communities/railOrder";
import { MentionBadge } from "@/features/mentions/MentionBadge";
import { mentionsText } from "@/features/mentions/mentions";
import { useMessages } from "@/i18n/context";
import {
  ChatsTeardropIcon,
  DotsSixVerticalIcon,
  GaugeIcon,
  GlobeSimpleIcon,
  PlusIcon,
} from "@phosphor-icons/react";
import {
  Button,
  DropIndicator,
  GridList,
  GridListItem,
  useDragAndDrop,
} from "react-aria-components";
import { format, type Messages } from "@/i18n/messages";
import { AddCommunityDialog } from "@/features/communities/AddCommunityDialog";
import { Avatar } from "@/features/communities/Avatar";
import { reorderIds } from "@/features/layout/reorder";
import { Tooltip } from "@/features/layout/Tooltip";

/** The drag type rail rows carry, so nothing else accepts them and they accept nothing else. */
const COMMUNITY_DRAG_TYPE = "application/x-aspen-community";

/** A community on the rail, from whichever deployment it belongs to. */
interface RailEntry {
  readonly key: string;
  readonly domain: string | null;
  readonly communityId: string;
  readonly community: Community;
  readonly source: Source;
  readonly unread: boolean;
  readonly tags: number;
}

/**
 * The narrow column of communities the user belongs to, on their home and every other
 * deployment they use, in the order they arranged them; another deployment's are marked with
 * its domain. Dragging a community moves it and the new order is saved to the user's home
 * preferences (`RAIL_ORDER`), and each deployment's own share of it to their memberships there:
 * with the pointer by dragging the avatar, with the keyboard through the handle that appears
 * on focus. It is a grid list rather than a list box for that handle, which list boxes cannot
 * carry.
 */
export function CommunityRail() {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const order = usePreference(RAIL_ORDER);
  // Shown at once while the new order is on its way to the server.
  const [moved, setMoved] = useState<readonly string[] | null>(null);
  const { entries, dmsUnread, dmTags } = useEverywhere(["communities", "unread"], (sources) => {
    const found: RailEntry[] = [];
    let unread = false;
    let tags = 0;
    for (const source of sources) {
      const store = source.sync.store;
      const places = store.unreadPlaces();
      unread ||= places.has(UNREAD_DMS);
      tags += store.placeMentions(UNREAD_DMS);
      for (const community of store.communities()) {
        const place = { domain: source.domain, communityId: community.id };
        found.push({
          ...place,
          key: railKey(place),
          community,
          source,
          unread: places.has(community.id),
          tags: store.placeMentions(community.id),
        });
      }
    }
    return { entries: found, dmsUnread: unread, dmTags: tags };
  });
  const communities = arrangeRail(entries, moved ?? order);
  const { communityId: current, domain: currentDomain } = useParams({ strict: false });
  const currentKey =
    current === undefined ? null : railKey({ domain: currentDomain ?? null, communityId: current });
  const matchRoute = useMatchRoute();
  const inDms =
    matchRoute({ to: "/dms", fuzzy: true }) !== false ||
    matchRoute({ to: "/at/$domain/dms", fuzzy: true }) !== false;
  const inAdmin = matchRoute({ to: "/admin" }) !== false;
  const admin = useIsAdmin();
  const { dragAndDropHooks } = useDragAndDrop({
    getItems: (keys) => Array.from(keys, (key) => ({ [COMMUNITY_DRAG_TYPE]: String(key) })),
    acceptedDragTypes: [COMMUNITY_DRAG_TYPE],
    onReorder: (event) => {
      const ordered = reorderIds(
        communities.map((c) => c.key),
        new Set(Array.from(event.keys, String)),
        { key: String(event.target.key), dropPosition: event.target.dropPosition },
      );
      setMoved(ordered);
      void sync.preferences
        .set(RAIL_ORDER, ordered)
        .catch(() => undefined)
        .finally(() => {
          setMoved(null);
        });
      // Each deployment keeps its own share of the order, for clients that show one at a time.
      const bySync = new Map<AspenSync, string[]>();
      for (const key of ordered) {
        const entry = communities.find((c) => c.key === key);
        if (entry !== undefined) {
          bySync.set(entry.source.sync, [
            ...(bySync.get(entry.source.sync) ?? []),
            entry.communityId,
          ]);
        }
      }
      for (const [owner, ids] of bySync) {
        void owner.reorderCommunities(ids).catch(() => undefined);
      }
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
        {dmsUnread && <UnreadDot />}
        <RailBadge count={dmTags} />
        <Tooltip text={m.dms.label}>
          <Link
            to="/dms"
            aria-label={placeLabel(
              m,
              dmsUnread ? format(m.unreadLabel, { name: m.dms.label }) : m.dms.label,
              dmTags,
            )}
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
        dependencies={[currentKey]}
        selectionMode="none"
        onAction={(key) => {
          const entry = communities.find((c) => c.key === String(key));
          if (entry !== undefined) {
            void navigate(communityLink(entry.domain, entry.communityId));
          }
        }}
        dragAndDropHooks={dragAndDropHooks}
        className="flex flex-col items-center gap-2 outline-none"
      >
        {(entry) => {
          const { community } = entry;
          const name =
            entry.domain === null
              ? community.name
              : format(m.deployments.communityAt, {
                  community: community.name,
                  domain: entry.domain,
                });
          return (
            <GridListItem
              id={entry.key}
              textValue={community.name}
              aria-label={placeLabel(
                m,
                entry.unread ? format(m.unreadLabel, { name }) : name,
                entry.tags,
              )}
              className={
                "group relative isolate cursor-pointer rounded-full outline-none focus-visible:ring-2 focus-visible:ring-accent/60 dragging:opacity-50 " +
                (entry.key === currentKey
                  ? "ring-2 ring-accent ring-offset-2 ring-offset-surface-rail"
                  : "")
              }
            >
              {entry.unread && <UnreadDot />}
              <RailBadge count={entry.tags} />
              {entry.domain !== null && <ForeignMark />}
              <SourceScope source={entry.source}>
                <Avatar name={community.name} iconId={community.icon} size="lg" />
              </SourceScope>
              {/* The handle keyboard and screen reader users drag with; it shows only on focus. */}
              <Button
                slot="drag"
                aria-label={format(m.dragCommunity, { community: community.name })}
                className="absolute -right-1 -bottom-1 rounded-full border border-line bg-surface-raised p-0.5 text-ink-faint opacity-0 outline-none focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent/50"
              >
                <DotsSixVerticalIcon size={12} aria-hidden="true" />
              </Button>
            </GridListItem>
          );
        }}
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
      {admin && (
        <>
          <div aria-hidden="true" className="h-px w-8 shrink-0 bg-line" />
          <Tooltip text={m.admin.open}>
            <Link
              to="/admin"
              aria-label={m.admin.open}
              aria-current={inAdmin ? "page" : undefined}
              className={
                "flex h-12 w-12 shrink-0 items-center justify-center rounded-full bg-surface-raised text-ink-muted outline-none hover:text-accent focus-visible:ring-2 focus-visible:ring-accent/60 " +
                (inAdmin
                  ? "text-accent ring-2 ring-accent ring-offset-2 ring-offset-surface-rail"
                  : "")
              }
            >
              <GaugeIcon size={22} aria-hidden="true" />
            </Link>
          </Tooltip>
        </>
      )}
    </nav>
  );
}

/**
 * The mark beside a rail entry that has something unread: a dot at its left edge, half tucked
 * under the entry's icon. It goes in an element that `isolate`s a stacking context, so its
 * negative z-index puts it beneath the icon without sending it behind the rail itself.
 */
/** A place's name for the rail, with its unread tags when it has any. */
function placeLabel(m: Messages, name: string, tags: number): string {
  return tags > 0 ? format(m.withMentions, { name, mentions: mentionsText(m, tags) }) : name;
}

/** The mark on another deployment's community: a globe over the top corner. */
function ForeignMark() {
  return (
    <span
      aria-hidden="true"
      className="pointer-events-none absolute -top-1 -right-1 z-10 rounded-full bg-surface-raised p-0.5 text-ink-muted ring-2 ring-surface-rail"
    >
      <GlobeSimpleIcon size={12} weight="bold" />
    </span>
  );
}

/** The count of unread tags over the corner of a rail entry. */
function RailBadge({ count }: { count: number }) {
  return (
    <MentionBadge
      count={count}
      className="pointer-events-none absolute -right-1 -bottom-1 z-10 ring-2 ring-surface-rail"
    />
  );
}

function UnreadDot() {
  return (
    <span
      aria-hidden="true"
      className="absolute top-1/2 -left-1 -z-10 h-2 w-2 -translate-y-1/2 rounded-full bg-ink"
    />
  );
}
