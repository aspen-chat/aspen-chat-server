import { groupChannels, type Category, type Channel, type Community } from "@aspen/protocol";
import {
  BellSlashIcon,
  CaretDownIcon,
  DotsSixVerticalIcon,
  HashIcon,
  ImageIcon,
  SpeakerHighIcon,
} from "@phosphor-icons/react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useRef, useState } from "react";
import {
  Button,
  DropIndicator,
  GridList,
  GridListItem,
  isTextDropItem,
  useDragAndDrop,
  type DropItem,
} from "react-aria-components";
import {
  useCategories,
  useChannels,
  useCollapsed,
  useMute,
  useShownWhenCollapsed,
  useSync,
  useUnread,
} from "@/api/hooks";
import { ChannelMenu, ChannelMenuButton } from "@/features/channels/ChannelMenu";
import { AddDialog } from "@/features/channels/AddDialog";
import { AddToCategoryDialog } from "@/features/channels/AddToCategoryDialog";
import { InviteDialog } from "@/features/invites/InviteDialog";
import { insertIds, reorderIds } from "@/features/layout/reorder";
import { Tooltip } from "@/features/layout/Tooltip";
import { SidebarFooter } from "@/features/layout/SidebarFooter";
import { IconPicker } from "@/features/media/IconPicker";
import { VoiceParticipants } from "@/features/voice/VoiceParticipants";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The drag type channel rows carry, so a channel can be dropped into any channel group but
 * nothing else accepts it and it accepts nothing else.
 */
const CHANNEL_DRAG_TYPE = "application/x-aspen-channel";

/** The community's channels, grouped by category, with the signed-in user's controls below. */
export function ChannelSidebar({ community }: { community: Community }) {
  const m = useMessages();
  const sync = useSync();
  const channels = useChannels(community.id);
  const categories = useCategories(community.id);
  const { topLevel, byCategory } = groupChannels(channels, categories);
  return (
    <div className="flex h-full flex-col border-r border-line bg-surface-raised">
      <div className="flex items-center gap-2 border-b border-line px-4 py-2">
        <h1 className="min-w-0 flex-1 truncate font-semibold">{community.name}</h1>
        <IconPicker
          onIcon={async (iconId) => {
            await sync.updateCommunity(community.id, { icon: iconId });
          }}
        >
          {(open, uploading) => (
            <Tooltip text={m.changeCommunityIcon}>
              <Button
                aria-label={m.changeCommunityIcon}
                onPress={open}
                isDisabled={uploading}
                className={headerButtonClass}
              >
                <ImageIcon size={18} aria-hidden="true" />
              </Button>
            </Tooltip>
          )}
        </IconPicker>
        <InviteDialog community={community} />
      </div>
      <nav aria-label={m.channelsLabel} className="flex-1 overflow-y-auto px-2 py-2">
        <ChannelGroup label={m.channelsLabel} parentCategory={null} channels={topLevel} />
        {categories.map((category) => (
          <CategorySection
            key={category.id}
            category={category}
            channels={byCategory.get(category.id) ?? []}
          />
        ))}
        <div className="mt-3 px-1">
          <AddDialog community={community} />
        </div>
      </nav>
      <SidebarFooter />
    </div>
  );
}

/**
 * A category's heading and channels. The heading folds the category away and back, for the
 * caller on all their devices; folded, it still shows the channel being viewed, unread ones,
 * and voice channels with someone in the call.
 */
function CategorySection({
  category,
  channels,
}: {
  category: Category;
  channels: readonly Channel[];
}) {
  const m = useMessages();
  const sync = useSync();
  const { channelId: current } = useParams({ strict: false });
  const collapsed = useCollapsed(category.id);
  const shown = useShownWhenCollapsed(collapsed ? channels.map((c) => c.id) : []);
  const visible = collapsed
    ? channels.filter((c) => c.id === current || shown.has(c.id))
    : channels;
  return (
    <section className="group mt-3">
      <div className="flex items-center gap-1 px-2 pb-1">
        <h2 className="min-w-0 flex-1">
          <Button
            aria-expanded={!collapsed}
            onPress={() => {
              void sync.setCategoryCollapsed(category.id, !collapsed).catch(() => undefined);
            }}
            className="flex w-full min-w-0 items-center gap-1 rounded text-left text-xs font-semibold tracking-wide text-ink-faint uppercase outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <CaretDownIcon
              size={10}
              weight="bold"
              aria-hidden="true"
              className={"shrink-0 transition-transform " + (collapsed ? "-rotate-90" : "")}
            />
            <span className="truncate">{category.name}</span>
          </Button>
        </h2>
        <AddToCategoryDialog category={category} />
      </div>
      <ChannelGroup
        label={format(m.categoryChannelsLabel, { category: category.name })}
        parentCategory={category.id}
        channels={visible}
        allIds={channels.map((c) => c.id)}
        collapsed={collapsed}
      />
    </section>
  );
}

/**
 * One group of channels: a category's, or the community's top-level ones. Text channels open
 * on activation; voice channels are listed but not yet usable. Dragging a channel, with the
 * pointer or the keyboard, reorders the group or moves the channel into another group, and
 * the new arrangement is saved. An empty group stays on screen so channels can be dropped
 * into it, unless it is a folded category, which never claims to be empty: its channels are
 * only out of view. A folded category shows only some of its channels, but positions are
 * worked out among all of them.
 */
function ChannelGroup({
  label,
  parentCategory,
  channels,
  allIds,
  collapsed = false,
}: {
  label: string;
  /** The category the group belongs to, or `null` for the top level. */
  parentCategory: string | null;
  /** The channels shown. */
  channels: readonly Channel[];
  /** Every channel of the group, in order, when some are not shown. */
  allIds?: readonly string[];
  /** Whether the group is a folded category. */
  collapsed?: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const { channelId: current } = useParams({ strict: false });
  const ids = allIds ?? channels.map((c) => c.id);
  /** The channel ids a drop from another group carries. */
  const droppedIds = (items: readonly DropItem[]) =>
    Promise.all(items.filter(isTextDropItem).map((item) => item.getText(CHANNEL_DRAG_TYPE))).then(
      (texts) => texts.filter((text) => text.length > 0),
    );
  const { dragAndDropHooks } = useDragAndDrop({
    getItems: (keys) => Array.from(keys, (key) => ({ [CHANNEL_DRAG_TYPE]: String(key) })),
    acceptedDragTypes: [CHANNEL_DRAG_TYPE],
    onReorder: (event) => {
      const moved = new Set(Array.from(event.keys, String));
      const ordered = reorderIds(ids, moved, {
        key: String(event.target.key),
        dropPosition: event.target.dropPosition,
      });
      void sync.arrangeChannels(ordered, parentCategory).catch(() => undefined);
    },
    onInsert: (event) => {
      void droppedIds(event.items)
        .then((arriving) =>
          sync.arrangeChannels(
            insertIds(ids, arriving, {
              key: String(event.target.key),
              dropPosition: event.target.dropPosition,
            }),
            parentCategory,
          ),
        )
        .catch(() => undefined);
    },
    onRootDrop: (event) => {
      void droppedIds(event.items)
        .then((arriving) => sync.arrangeChannels([...ids, ...arriving], parentCategory))
        .catch(() => undefined);
    },
    renderDropIndicator: (target) => (
      <DropIndicator
        target={target}
        className="mx-2 h-0.5 rounded-full bg-transparent drop-target:bg-accent"
      />
    ),
  });
  return (
    <GridList
      aria-label={label}
      items={channels}
      // Each row's rendering is cached by its channel; the highlight on the current channel
      // comes from the route, so the route is declared as a dependency.
      dependencies={[current]}
      selectionMode="none"
      renderEmptyState={() =>
        collapsed ? null : (
          <p className="rounded-md border border-dashed border-line px-2 py-1 text-xs text-ink-faint">
            {m.emptyChannelGroup}
          </p>
        )
      }
      onAction={(key) => {
        const channel = channels.find((c) => c.id === key);
        if (channel?.ty === "text") {
          void navigate({
            to: "/communities/$communityId/channels/$channelId",
            params: { communityId: channel.community ?? "", channelId: channel.id },
          });
        } else if (channel?.ty === "voice") {
          void navigate({
            to: "/communities/$communityId/channels/$channelId",
            params: { communityId: channel.community ?? "", channelId: channel.id },
          });
          void sync.voice.join(channel.id).catch(() => undefined);
        }
      }}
      dragAndDropHooks={dragAndDropHooks}
      className="flex flex-col gap-0.5 outline-none"
    >
      {(channel) => (
        <GridListItem
          id={channel.id}
          textValue={channel.name}
          className={
            "group flex flex-wrap items-center gap-1.5 rounded-md px-2 py-1 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 dragging:opacity-50 " +
            "cursor-pointer text-ink-muted hover:bg-surface-hover hover:text-ink " +
            (channel.id === current ? "bg-surface-hover font-medium text-ink" : "")
          }
        >
          <ChannelLabel channel={channel} current={channel.id === current} />
          {/* The handle keyboard and screen reader users drag with; pointer users drag the row. */}
          <Button
            slot="drag"
            aria-label={format(m.dragChannel, { channel: channel.name })}
            className="ml-auto rounded p-0.5 text-ink-faint opacity-0 outline-none group-hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <DotsSixVerticalIcon size={14} aria-hidden="true" />
          </Button>
          {channel.ty === "voice" && (
            <div className="basis-full">
              <VoiceParticipants channelId={channel.id} />
            </div>
          )}
        </GridListItem>
      )}
    </GridList>
  );
}

const headerButtonClass =
  "rounded-md border border-line p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * How an unread channel or DM is marked in its list: brighter, inside a rounded accent outline
 * over a faint accent fill. Where it goes supplies padding that, with the 1px border, makes up
 * the row's own padding, so nothing moves when the mark comes and goes.
 */
export const unreadMarkClass = "rounded-md border border-accent bg-accent/20 text-ink";

/**
 * A channel's icon and name in the list. While it holds something the caller has not read they
 * are marked together, and the label takes all the width up to the drag handle, so every unread
 * channel's mark is as wide as the next. The mark fills the row's padding: 3px and the border
 * make its 4px top and bottom, 6px and the border its 8px sides, less the 1px the row keeps at
 * its edges. A muted channel is dimmed, carries a muted bell, and is never marked unread. A text
 * channel's menu opens on a right click or from its options button.
 */
function ChannelLabel({ channel, current }: { channel: Channel; current: boolean }) {
  const m = useMessages();
  const unread = useUnread(channel.id);
  const muted = useMute(channel.id) !== undefined;
  const label = useRef<HTMLSpanElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const marked = unread && !muted && !current;
  const hasMenu = channel.ty === "text";
  const accessibleName = muted
    ? format(m.mutedLabel, { name: channel.name })
    : unread
      ? format(m.unreadLabel, { name: channel.name })
      : null;
  return (
    <span
      ref={label}
      onContextMenu={(event) => {
        if (hasMenu) {
          event.preventDefault();
          setMenuOpen(true);
        }
      }}
      className={
        "flex min-w-0 flex-1 items-center gap-1.5" +
        (marked ? " -mx-[7px] -my-1 px-1.5 py-[3px] " + unreadMarkClass : "") +
        (muted && !current ? " text-ink-faint" : "")
      }
    >
      {channel.ty === "text" ? (
        <HashIcon size={16} aria-hidden="true" className="shrink-0 text-ink-faint" />
      ) : (
        <SpeakerHighIcon size={16} aria-hidden="true" className="shrink-0" />
      )}
      {accessibleName === null ? (
        <span className="truncate">{channel.name}</span>
      ) : (
        <>
          <span aria-hidden="true" className="truncate">
            {channel.name}
          </span>
          <span className="sr-only">{accessibleName}</span>
        </>
      )}
      {muted && <BellSlashIcon size={14} aria-hidden="true" className="ml-auto shrink-0" />}
      {hasMenu && (
        <>
          <ChannelMenuButton
            name={channel.name}
            isOpen={menuOpen}
            onPress={() => {
              setMenuOpen((open) => !open);
            }}
            className={muted ? "" : "ml-auto"}
          />
          <ChannelMenu
            channelId={channel.id}
            name={channel.name}
            anchorRef={label}
            isOpen={menuOpen}
            onOpenChange={setMenuOpen}
          />
        </>
      )}
    </span>
  );
}
