import { Link, useMatchRoute, useNavigate, useParams } from "@tanstack/react-router";
import { useReorderGlide } from "@/features/layout/motion";
import {
  RAIL_FOLDERS,
  RAIL_ORDER,
  UNREAD_DMS,
  preferenceValue,
  type AspenSync,
  type Community,
  type FolderColor,
  type RailFolder,
} from "@aspen/protocol";
import { useRef, useState } from "react";
import { SourceScope } from "@/api/deployments";
import { useEverywhere, type Source } from "@/api/everywhere";
import { useIsAdmin, usePreference, useSync, useSyncStatus } from "@/api/hooks";
import { communityLink } from "@/features/messages/links";
import {
  arrangeRail,
  folderIdOf,
  folderKey,
  moveInRail,
  railKey,
  railSequence,
  ungroupFolder,
  updateFolder,
  type RailDrop,
  type RailLayout,
  type RailUnit,
} from "@/features/communities/railOrder";
import {
  FolderMenu,
  FolderOptionsButton,
  FolderTile,
  RenameFolderDialog,
} from "@/features/communities/RailFolder";
import { FOLDER_TINT, folderName } from "@/features/communities/folders";
import { MentionBadge } from "@/features/mentions/MentionBadge";
import { mentionsText } from "@/features/mentions/mentions";
import { useMessages } from "@/i18n/context";
import {
  ChatsTeardropIcon,
  DotsSixVerticalIcon,
  DotsThreeIcon,
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
import { Tooltip } from "@/features/layout/Tooltip";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";

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
 * One row of the rail's list: a community, standing alone or on its open folder's band (the
 * band's last row rounds it off), or a folder's own row.
 */
type RailRow =
  | {
      readonly kind: "community";
      readonly key: string;
      readonly entry: RailEntry;
      readonly band: {
        readonly folder: string;
        readonly color: FolderColor;
        readonly last: boolean;
      } | null;
    }
  | {
      readonly kind: "folder";
      readonly key: string;
      readonly folder: RailFolder;
      readonly entries: readonly RailEntry[];
    };

function railRows(units: readonly RailUnit<RailEntry>[]): RailRow[] {
  return units.flatMap((unit): RailRow[] => {
    if (unit.kind === "community") {
      return [{ kind: "community", key: unit.entry.key, entry: unit.entry, band: null }];
    }
    const { folder, entries } = unit;
    const header: RailRow = { kind: "folder", key: folderKey(folder.id), folder, entries };
    if (!folder.open) {
      return [header];
    }
    return [
      header,
      ...entries.map((entry, index): RailRow => ({
        kind: "community",
        key: entry.key,
        entry,
        band: { folder: folder.id, color: folder.color, last: index === entries.length - 1 },
      })),
    ];
  });
}

const currentRing = "ring-2 ring-accent ring-offset-2 ring-offset-surface-rail";
/** The focus ring and the mark of a drop onto a row, drawn on its tile rather than its band. */
const tileStateClass =
  "group-data-[focus-visible]:ring-2 group-data-[focus-visible]:ring-accent/60 " +
  "group-data-[drop-target]:ring-2 group-data-[drop-target]:ring-accent";
const handleClass =
  "absolute -end-1 -bottom-1 rounded-full border border-line bg-surface-raised p-0.5 text-ink-faint opacity-0 outline-none focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The narrow column of communities the user belongs to, on their home and every other
 * deployment they use, in the order they arranged them; another deployment's are marked with
 * its domain. Communities may be gathered into folders, as on an iPhone's home screen:
 * dropping one community on another makes a folder of the two, dropping one on a folder adds
 * it, and a folder left with one community goes. A closed folder is a tile of its first four
 * icons carrying its communities' unread mark and tag count; pressing it opens it in place,
 * its communities standing on its tint below it, where they are dragged within it or out of
 * it (after its last). Its menu, from a right click or its options button, renames, tints,
 * and ungroups it. The arrangement, folders, and which are open follow the account
 * (`RAIL_ORDER`, `RAIL_FOLDERS`, written together), and each deployment also gets its own
 * share of the order for its memberships there. Dragging works with the pointer on the tile
 * and with the keyboard through the handle that appears on focus. It is a grid list rather
 * than a list box for those handles, which list boxes cannot carry.
 */
export function CommunityRail() {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const order = usePreference(RAIL_ORDER);
  const folders = usePreference(RAIL_FOLDERS);
  // Shown at once while the new arrangement is on its way to the server.
  const [moved, setMoved] = useState<RailLayout | null>(null);
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
  const units = arrangeRail(entries, moved ?? { order, folders });
  const rows = railRows(units);
  const { communityId: current, domain: currentDomain } = useParams({ strict: false });
  const currentKey =
    current === undefined ? null : railKey({ domain: currentDomain ?? null, communityId: current });
  const matchRoute = useMatchRoute();
  const inDms =
    matchRoute({ to: "/dms", fuzzy: true }) !== false ||
    matchRoute({ to: "/at/$domain/dms", fuzzy: true }) !== false;
  const inAdmin = matchRoute({ to: "/admin", fuzzy: true }) !== false;
  const admin = useIsAdmin();
  const bootstrapping = useSyncStatus() === "bootstrapping";
  /** The row being dragged, which decides what it may be dropped on. */
  const dragging = useRef<string | null>(null);
  const menuAnchor = useRef<HTMLElement | null>(null);
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const list = useRef<HTMLDivElement>(null);
  useReorderGlide(list, rows.map((row) => row.key).join(" "));
  /** The folders opened here since the rail was drawn, whose communities drop into place. */
  const [openedHere, setOpenedHere] = useState<ReadonlySet<string>>(new Set());
  const [renaming, setRenaming] = useState<string | null>(null);

  function save(layout: RailLayout, reordered: boolean) {
    setMoved(layout);
    void sync.preferences
      .setAccount(
        preferenceValue(RAIL_ORDER, layout.order),
        preferenceValue(RAIL_FOLDERS, layout.folders),
      )
      .catch(() => undefined)
      .finally(() => {
        setMoved(null);
      });
    if (!reordered) {
      return;
    }
    // Each deployment keeps its own share of the order, folders opened in place, for clients
    // that show one at a time.
    const byKey = new Map(entries.map((entry) => [entry.key, entry]));
    const bySync = new Map<AspenSync, string[]>();
    for (const key of railSequence(arrangeRail(entries, layout))) {
      const entry = byKey.get(key);
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
  }

  function drop(dragged: string, target: RailDrop) {
    const layout = moveInRail(units, dragged, target, () => crypto.randomUUID());
    if (layout !== null) {
      save(layout, true);
    }
  }

  const { dragAndDropHooks } = useDragAndDrop({
    getItems: (keys) => Array.from(keys, (key) => ({ [COMMUNITY_DRAG_TYPE]: String(key) })),
    acceptedDragTypes: [COMMUNITY_DRAG_TYPE],
    onDragStart: (event) => {
      dragging.current = Array.from(event.keys, String)[0] ?? null;
    },
    onDragEnd: () => {
      dragging.current = null;
    },
    // Only a community goes onto something, and never a folder into a folder.
    shouldAcceptItemDrop: (target) => {
      const dragged = dragging.current;
      return dragged !== null && folderIdOf(dragged) === null && String(target.key) !== dragged;
    },
    onItemDrop: (event) => {
      const dragged = dragging.current;
      if (dragged !== null) {
        drop(dragged, { key: String(event.target.key), position: "on" });
      }
    },
    onReorder: (event) => {
      const dragged = Array.from(event.keys, String)[0];
      if (dragged !== undefined && event.target.dropPosition !== "on") {
        drop(dragged, { key: String(event.target.key), position: event.target.dropPosition });
      }
    },
    renderDropIndicator: (target) => (
      <DropIndicator
        target={target}
        className="h-0.5 w-12 rounded-full bg-transparent drop-target:bg-accent"
      />
    ),
  });
  const menuFolder = folders.find((f) => f.id === menuFor) ?? null;
  const renameFolder = folders.find((f) => f.id === renaming) ?? null;
  return (
    <nav
      aria-label={m.communitiesLabel}
      className="flex w-16 shrink-0 flex-col items-center gap-2 overflow-y-auto border-e border-line bg-surface-rail py-3"
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
              (inDms ? "text-accent " + currentRing : "")
            }
          >
            <ChatsTeardropIcon size={22} aria-hidden="true" />
          </Link>
        </Tooltip>
      </div>
      <div aria-hidden="true" className="h-px w-8 bg-line" />
      {rows.length === 0 && bootstrapping && (
        <div aria-busy="true" className="flex flex-col items-center gap-2 py-1">
          <LoadingLabel />
          {[0, 1, 2].map((index) => (
            <Skeleton key={index} className="h-12 w-12 rounded-full" />
          ))}
        </div>
      )}
      <GridList
        ref={list}
        aria-label={m.communitiesLabel}
        items={rows}
        // The list caches each item's rendering by its data; the ring around the current
        // community comes from the route, so the route is declared as a dependency.
        dependencies={[currentKey, openedHere]}
        selectionMode="none"
        onAction={(key) => {
          const id = folderIdOf(String(key));
          if (id !== null) {
            const folder = folders.find((f) => f.id === id);
            if (folder !== undefined) {
              setOpenedHere((opened) => {
                const next = new Set(opened);
                if (folder.open) {
                  next.delete(id);
                } else {
                  next.add(id);
                }
                return next;
              });
              save(updateFolder(units, id, { open: !folder.open }), false);
            }
            return;
          }
          const entry = entries.find((c) => c.key === String(key));
          if (entry !== undefined) {
            void navigate(communityLink(entry.domain, entry.communityId));
          }
        }}
        dragAndDropHooks={dragAndDropHooks}
        className="flex flex-col items-center outline-none"
      >
        {(row) =>
          row.kind === "folder" ? (
            <FolderRow
              row={row}
              currentKey={currentKey}
              onMenu={(anchor) => {
                menuAnchor.current = anchor;
                setMenuFor(row.folder.id);
              }}
            />
          ) : (
            <CommunityRow
              row={row}
              current={row.key === currentKey}
              arriving={row.band !== null && openedHere.has(row.band.folder)}
            />
          )
        }
      </GridList>
      {menuFolder !== null && (
        <FolderMenu
          folder={menuFolder}
          anchorRef={menuAnchor}
          isOpen
          onOpenChange={(open) => {
            if (!open) {
              setMenuFor(null);
            }
          }}
          onRename={() => {
            setRenaming(menuFolder.id);
          }}
          onColor={(color) => {
            save(updateFolder(units, menuFolder.id, { color }), false);
          }}
          onUngroup={() => {
            save(ungroupFolder(units, menuFolder.id), true);
          }}
        />
      )}
      {renameFolder !== null && (
        <RenameFolderDialog
          key={renameFolder.id}
          folder={renameFolder}
          isOpen
          onOpenChange={(open) => {
            if (!open) {
              setRenaming(null);
            }
          }}
          onSave={(name) => {
            save(updateFolder(units, renameFolder.id, { name }), false);
          }}
        />
      )}
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

function CommunityRow({
  row,
  current,
  arriving,
}: {
  row: Extract<RailRow, { kind: "community" }>;
  current: boolean;
  /** Whether it is in a folder the reader just opened, and drops into place. */
  arriving: boolean;
}) {
  const m = useMessages();
  const { entry, band } = row;
  const { community } = entry;
  const name =
    entry.domain === null
      ? community.name
      : format(m.deployments.communityAt, { community: community.name, domain: entry.domain });
  return (
    <GridListItem
      id={row.key}
      textValue={community.name}
      aria-label={placeLabel(m, entry.unread ? format(m.unreadLabel, { name }) : name, entry.tags)}
      className={
        "group flex w-14 cursor-pointer justify-center py-1 outline-none dragging:opacity-50 " +
        (arriving ? "motion-drop " : "") +
        (band === null ? "" : FOLDER_TINT[band.color] + (band.last ? " rounded-b-2xl pb-2" : ""))
      }
    >
      <div
        className={
          "relative isolate rounded-full " + tileStateClass + (current ? " " + currentRing : "")
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
          className={handleClass}
        >
          <DotsSixVerticalIcon size={12} aria-hidden="true" />
        </Button>
      </div>
    </GridListItem>
  );
}

function FolderRow({
  row,
  currentKey,
  onMenu,
}: {
  row: Extract<RailRow, { kind: "folder" }>;
  currentKey: string | null;
  onMenu: (anchor: HTMLElement) => void;
}) {
  const m = useMessages();
  const { folder, entries } = row;
  const name = folderName(m, folder);
  const count = String(entries.length);
  // Closed, it speaks for its communities: their unread mark, their tags, the current one.
  const unread = !folder.open && entries.some((entry) => entry.unread);
  const tags = folder.open ? 0 : entries.reduce((sum, entry) => sum + entry.tags, 0);
  const current = !folder.open && entries.some((entry) => entry.key === currentKey);
  const one = entries.length === 1;
  const label = format(
    folder.open
      ? one
        ? m.folders.labelOpenOne
        : m.folders.labelOpen
      : one
        ? m.folders.labelOne
        : m.folders.label,
    { name, count },
  );
  const tile = useRef<HTMLDivElement>(null);
  return (
    <GridListItem
      id={row.key}
      textValue={name}
      aria-label={placeLabel(m, unread ? format(m.unreadLabel, { name: label }) : label, tags)}
      className={
        "group flex w-14 cursor-pointer justify-center pt-1 outline-none dragging:opacity-50 " +
        (folder.open ? FOLDER_TINT[folder.color] + " rounded-t-2xl pb-1" : "pb-1")
      }
    >
      <div
        ref={tile}
        onContextMenu={(event) => {
          event.preventDefault();
          onMenu(event.currentTarget);
        }}
        className={
          "relative isolate rounded-2xl " + tileStateClass + (current ? " " + currentRing : "")
        }
      >
        {unread && <UnreadDot />}
        <RailBadge count={tags} />
        <FolderTile
          folder={folder}
          members={entries.map((entry) => ({
            key: entry.key,
            name: entry.community.name,
            icon: entry.community.icon,
            source: entry.source,
          }))}
        />
        <FolderOptionsButton
          label={format(m.folders.options, { name })}
          onPress={() => {
            if (tile.current !== null) {
              onMenu(tile.current);
            }
          }}
        >
          <DotsThreeIcon size={12} aria-hidden="true" />
        </FolderOptionsButton>
        <Button slot="drag" aria-label={format(m.folders.drag, { name })} className={handleClass}>
          <DotsSixVerticalIcon size={12} aria-hidden="true" />
        </Button>
      </div>
    </GridListItem>
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
      className="pointer-events-none absolute -top-1 -end-1 z-10 rounded-full bg-surface-raised p-0.5 text-ink-muted ring-2 ring-surface-rail"
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
      className="pointer-events-none absolute -end-1 -bottom-1 z-10 ring-2 ring-surface-rail"
    />
  );
}

function UnreadDot() {
  return (
    <span
      aria-hidden="true"
      className="motion-grow absolute top-1/2 -start-1 -z-10 h-2 w-2 -translate-y-1/2 rounded-full bg-ink"
    />
  );
}
