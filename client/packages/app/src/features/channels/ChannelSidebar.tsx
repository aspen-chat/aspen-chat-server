import { groupChannels, type Category, type Channel, type Community } from "@aspen/protocol";
import { useGrowthKey, useReorderGlide } from "@/features/layout/motion";
import { PaneEdge } from "@/features/layout/ResizablePane";
import {
  CaretDownIcon,
  DotsSixVerticalIcon,
  HashIcon,
  LockSimpleIcon,
  SpeakerHighIcon,
} from "@phosphor-icons/react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { PluginGlyph } from "@/features/plugins/PluginChannel";
import { useId, useRef, useState } from "react";
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
  useAccess,
  useCan,
  useDeploymentCan,
  useCategories,
  useMe,
  useMemberRoles,
  usePluginKind,
  useChannels,
  useCollapsed,
  useMute,
  useShownWhenCollapsed,
  useSync,
  useMentions,
  useUnread,
} from "@/api/hooks";
import { DeleteChannelDialog, RenameChannelDialog } from "@/features/channels/ChannelDialogs";
import { ChannelMenu, ChannelMenuButton } from "@/features/channels/ChannelMenu";
import { MuteBell } from "@/features/channels/MuteBell";
import { AddDialog } from "@/features/channels/AddDialog";
import { AddToCategoryDialog } from "@/features/channels/AddToCategoryDialog";
import { AccessDialog } from "@/features/community-settings/AccessDialog";
import { CommunitySettingsDialog } from "@/features/community-settings/CommunitySettingsDialog";
import { InviteDialog } from "@/features/invites/InviteDialog";
import { insertIds, reorderIds } from "@/features/layout/reorder";
import { headerIconButtonClass } from "@/features/layout/headerButton";
import { Tooltip } from "@/features/layout/Tooltip";
import { SidebarFooter } from "@/features/layout/SidebarFooter";
import { SidebarHeader } from "@/features/layout/SidebarHeader";
import { VoiceParticipants } from "@/features/voice/VoiceParticipants";
import { MentionBadge } from "@/features/mentions/MentionBadge";
import { mentionsText } from "@/features/mentions/mentions";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useDomain, channelLink } from "@/features/messages/links";
import { CopyIdButton } from "@/features/layout/CopyId";
import { OnceOpen } from "@/features/layout/OnceOpen";

/**
 * The drag type channel rows carry, so a channel can be dropped into any channel group but
 * nothing else accepts it and it accepts nothing else.
 */
const CHANNEL_DRAG_TYPE = "application/x-aspen-channel";

/** The community's channels, grouped by category, with the signed-in user's controls below. */
export function ChannelSidebar({ community }: { community: Community }) {
  const m = useMessages();
  const headingId = useId();
  const channels = useChannels(community.id);
  const categories = useCategories(community.id);
  const createInvites = useCan(community.id, "createInvites");
  const me = useMe();
  // Someone here only by moderating the server holds no roles in it, not even everyone's.
  const held = useMemberRoles(community.id, me?.id ?? "");
  const access = useAccess(community.id);
  const moderating = held === undefined && (access?.moderator ?? false);
  const manageInvites = useCan(community.id, "manageInvites");
  const { topLevel, byCategory } = groupChannels(channels, categories);
  return (
    // A landmark named by its heading, holding the channels and the user's own controls.
    <section
      aria-labelledby={headingId}
      className="relative flex h-full flex-col border-e border-line bg-surface-raised"
    >
      <SidebarHeader headingId={headingId} title={community.name}>
        <CommunitySettingsDialog community={community} triggerClassName={headerIconButtonClass} />
        {(createInvites || manageInvites) && <InviteDialog community={community} />}
        <CopyIdButton id={community.id} thing="community" className="p-1.5" />
      </SidebarHeader>
      {moderating && (
        <p className="border-b border-line bg-accent-soft px-4 py-2 text-xs text-accent-strong">
          {m.communitySettings.moderatorNote}
        </p>
      )}
      <nav aria-label={m.channelsLabel} className="flex-1 overflow-y-auto px-2 py-2">
        <ChannelGroup
          communityId={community.id}
          label={m.channelsLabel}
          parentCategory={null}
          channels={topLevel}
        />
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
      <PaneEdge />
    </section>
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
  // Unfolded since it was first drawn: the channels it reveals drop into place.
  const unfolded = useGrowthKey(collapsed ? 0 : 1) > 0;
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
            className="flex w-full min-w-0 items-center gap-1 rounded text-start text-xs font-semibold tracking-wide text-ink-faint uppercase outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
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
        <CategoryAccessButton category={category} />
        <AddToCategoryDialog category={category} />
        <CopyIdButton id={category.id} thing="category" className="p-0.5" />
      </div>
      <ChannelGroup
        communityId={category.community}
        label={format(m.categoryChannelsLabel, { category: category.name })}
        parentCategory={category.id}
        channels={visible}
        allIds={channels.map((c) => c.id)}
        collapsed={collapsed}
        unfolded={unfolded}
      />
    </section>
  );
}

/** The control on a category's heading that opens who can use its channels. */
function CategoryAccessButton({ category }: { category: Category }) {
  const m = useMessages();
  const manage = useCan(category.community, "manageCategories");
  const [open, setOpen] = useState(false);
  if (!manage) {
    return null;
  }
  const label = format(m.access.categoryHeading, { category: category.name });
  return (
    <>
      <Tooltip text={label}>
        <Button
          aria-label={label}
          onPress={() => {
            setOpen(true);
          }}
          className="tap-target rounded p-0.5 text-ink-faint opacity-0 outline-none group-hover:opacity-100 pointer-coarse:opacity-100 hover:bg-surface-hover hover:text-ink focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <LockSimpleIcon size={14} aria-hidden="true" />
        </Button>
      </Tooltip>
      <AccessDialog
        target={{
          kind: "category",
          id: category.id,
          name: category.name,
          communityId: category.community,
        }}
        isOpen={open}
        onOpenChange={setOpen}
      />
    </>
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
  communityId,
  label,
  parentCategory,
  channels,
  allIds,
  collapsed = false,
  unfolded = false,
}: {
  communityId: string;
  label: string;
  /** The category the group belongs to, or `null` for the top level. */
  parentCategory: string | null;
  /** The channels shown. */
  channels: readonly Channel[];
  /** Every channel of the group, in order, when some are not shown. */
  allIds?: readonly string[];
  /** Whether the group is a folded category. */
  collapsed?: boolean;
  /** Whether it is a category unfolded since it was drawn, whose channels drop into place. */
  unfolded?: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const domain = useDomain();
  const { channelId: current } = useParams({ strict: false });
  const arrange = useCan(communityId, "manageChannels");
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
        className="mx-2 h-0.5 rounded-full bg-transparent drop-target:bg-accent drop-target:forced-fill"
      />
    ),
  });
  const list = useRef<HTMLDivElement>(null);
  useReorderGlide(list, channels.map((c) => c.id).join(" "));
  return (
    <GridList
      // The list calls more hooks while it has drag and drop hooks, so it is a new list when
      // they come or go (the sidebar moving to a community where the reader may arrange
      // channels, or leaving one) rather than one whose hook order changes.
      key={arrange ? "arrange" : "view"}
      ref={list}
      aria-label={label}
      items={channels}
      // Each row's rendering is cached by its channel; the highlight on the current channel
      // comes from the route, so the route is declared as a dependency.
      dependencies={[current, unfolded]}
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
        if (channel?.ty === "text" || channel?.ty === "plugin") {
          void navigate(channelLink({ domain, community: channel.community ?? "" }, channel.id));
        } else if (channel?.ty === "voice") {
          void navigate(channelLink({ domain, community: channel.community ?? "" }, channel.id));
          if (sync.store.channelAccess(channel.id).has("joinVoice")) {
            void sync.voice.join(channel.id).catch(() => undefined);
          }
        }
      }}
      // Only someone who may manage channels can move them.
      {...(arrange ? { dragAndDropHooks } : {})}
      className="flex flex-col gap-0.5 outline-none"
    >
      {(channel) => (
        <GridListItem
          id={channel.id}
          textValue={channel.name}
          className={
            "group flex flex-wrap items-center gap-1.5 rounded-md px-2 py-1 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 dragging:opacity-50 " +
            "cursor-pointer text-ink-muted hover:bg-surface-hover hover:text-ink " +
            // The current channel stays in view while folded; it has nowhere to drop from.
            (unfolded && channel.id !== current ? "motion-drop " : "") +
            (channel.id === current ? "bg-surface-hover font-medium text-ink" : "")
          }
        >
          <ChannelLabel channel={channel} current={channel.id === current} />
          {/* The handle keyboard and screen reader users drag with; pointer users drag the row. */}
          {arrange && (
            <Button
              slot="drag"
              aria-label={format(m.dragChannel, { channel: channel.name })}
              className="ms-auto rounded p-0.5 text-ink-faint opacity-0 outline-none group-hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              <DotsSixVerticalIcon size={14} aria-hidden="true" />
            </Button>
          )}
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
 * its edges. A muted channel is dimmed, carries a muted bell whose tooltip says until when, and
 * is never marked unread, though the count of unread messages that tag the reader shows on it
 * as on any other. The menu of a text channel, or of one a plugin shows, opens on a right click
 * or from its options button.
 */
function ChannelLabel({ channel, current }: { channel: Channel; current: boolean }) {
  const m = useMessages();
  const unread = useUnread(channel.id);
  const tags = useMentions(channel.id);
  const mute = useMute(channel.id);
  const muted = mute !== undefined;
  const manage = useCan(channel.community, "manageChannels");
  // A moderator of the server may rename and delete any channel, and nothing else here.
  const moderator = useDeploymentCan("moderateCommunities");
  const kind = usePluginKind(channel);
  const label = useRef<HTMLSpanElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [dialog, setDialog] = useState<"access" | "rename" | "delete" | null>(null);
  const marked = unread && !muted && !current;
  const hasMenu = channel.ty === "text" || channel.ty === "plugin" || manage || moderator;
  const close = (open: boolean) => {
    if (!open) {
      setDialog(null);
    }
  };
  const stateName = muted
    ? format(m.mutedLabel, { name: channel.name })
    : unread
      ? format(m.unreadLabel, { name: channel.name })
      : null;
  const accessibleName =
    tags > 0
      ? format(m.withMentions, {
          name: stateName ?? channel.name,
          mentions: mentionsText(m, tags),
        })
      : stateName;
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
      ) : channel.ty === "plugin" ? (
        <PluginGlyph glyph={kind?.kind.glyph} size={16} className="shrink-0 text-ink-faint" />
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
      <MentionBadge count={tags} className="ms-auto" />
      {mute !== undefined && <MuteBell mute={mute} className={tags > 0 ? "" : "ms-auto"} />}
      {hasMenu && (
        <>
          <ChannelMenuButton
            name={channel.name}
            isOpen={menuOpen}
            onPress={() => {
              setMenuOpen((open) => !open);
            }}
            className={muted || tags > 0 ? "" : "ms-auto"}
          />
          <OnceOpen isOpen={menuOpen}>
            <ChannelMenu
              channelId={channel.id}
              name={channel.name}
              anchorRef={label}
              isOpen={menuOpen}
              onOpenChange={setMenuOpen}
              mutable={channel.ty === "text" || channel.ty === "plugin"}
              {...(manage
                ? {
                    onAccess: () => {
                      setDialog("access");
                    },
                  }
                : {})}
              {...(manage || moderator
                ? {
                    onRename: () => {
                      setDialog("rename");
                    },
                    onDelete: () => {
                      setDialog("delete");
                    },
                  }
                : {})}
            />
          </OnceOpen>
        </>
      )}
      {manage && channel.community != null && (
        <AccessDialog
          target={{
            kind: "channel",
            id: channel.id,
            name: channel.name,
            communityId: channel.community,
          }}
          isOpen={dialog === "access"}
          onOpenChange={close}
        />
      )}
      {(manage || moderator) && (
        <>
          <RenameChannelDialog
            channel={channel}
            isOpen={dialog === "rename"}
            onOpenChange={close}
          />
          <DeleteChannelDialog
            channel={channel}
            isOpen={dialog === "delete"}
            onOpenChange={close}
          />
        </>
      )}
    </span>
  );
}
