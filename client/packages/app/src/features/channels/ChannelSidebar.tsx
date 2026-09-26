import { groupChannels, type Channel, type Community } from "@aspen/protocol";
import {
  DotsSixVerticalIcon,
  HashIcon,
  ImageIcon,
  SignOutIcon,
  SpeakerHighIcon,
} from "@phosphor-icons/react";
import { useNavigate, useParams } from "@tanstack/react-router";
import {
  Button,
  DropIndicator,
  GridList,
  GridListItem,
  isTextDropItem,
  useDragAndDrop,
  type DropItem,
} from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { useCategories, useChannels, useMe, useSync } from "@/api/hooks";
import { AddDialog } from "@/features/channels/AddDialog";
import { AddToCategoryDialog } from "@/features/channels/AddToCategoryDialog";
import { Avatar } from "@/features/communities/Avatar";
import { InviteDialog } from "@/features/invites/InviteDialog";
import { insertIds, reorderIds } from "@/features/layout/reorder";
import { Tooltip } from "@/features/layout/Tooltip";
import { IconPicker } from "@/features/media/IconPicker";
import { EditProfileDialog } from "@/features/users/EditProfileDialog";
import { CallBar } from "@/features/voice/CallBar";
import { VoiceEndedDialog } from "@/features/voice/VoiceEndedDialog";
import { VoiceParticipants } from "@/features/voice/VoiceParticipants";
import { displayNameOf, statusLine } from "@/features/users/profile";
import { ThemePicker } from "@/theme/ThemePicker";
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
          <section key={category.id} className="group mt-3">
            <div className="flex items-center gap-1 px-2 pb-1">
              <h2 className="min-w-0 flex-1 truncate text-xs font-semibold tracking-wide text-ink-faint uppercase">
                {category.name}
              </h2>
              <AddToCategoryDialog category={category} />
            </div>
            <ChannelGroup
              label={format(m.categoryChannelsLabel, { category: category.name })}
              parentCategory={category.id}
              channels={byCategory.get(category.id) ?? []}
            />
          </section>
        ))}
        <div className="mt-3 px-1">
          <AddDialog community={community} />
        </div>
      </nav>
      <CallBar />
      <UserFooter />
      <VoiceEndedDialog />
    </div>
  );
}

/**
 * One group of channels: a category's, or the community's top-level ones. Text channels open
 * on activation; voice channels are listed but not yet usable. Dragging a channel, with the
 * pointer or the keyboard, reorders the group or moves the channel into another group, and
 * the new arrangement is saved. An empty group stays on screen so channels can be dropped
 * into it.
 */
function ChannelGroup({
  label,
  parentCategory,
  channels,
}: {
  label: string;
  /** The category the group belongs to, or `null` for the top level. */
  parentCategory: string | null;
  channels: readonly Channel[];
}) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const { channelId: current } = useParams({ strict: false });
  const ids = channels.map((c) => c.id);
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
      selectionMode="none"
      renderEmptyState={() => (
        <p className="rounded-md border border-dashed border-line px-2 py-1 text-xs text-ink-faint">
          {m.emptyChannelGroup}
        </p>
      )}
      onAction={(key) => {
        const channel = channels.find((c) => c.id === key);
        if (channel?.ty === "Text") {
          void navigate({
            to: "/communities/$communityId/channels/$channelId",
            params: { communityId: channel.community ?? "", channelId: channel.id },
          });
        } else if (channel?.ty === "Voice") {
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
          {channel.ty === "Text" ? (
            <HashIcon size={16} aria-hidden="true" className="shrink-0 text-ink-faint" />
          ) : (
            <SpeakerHighIcon size={16} aria-hidden="true" className="shrink-0" />
          )}
          <span className="truncate">{channel.name}</span>
          {/* The handle keyboard and screen reader users drag with; pointer users drag the row. */}
          <Button
            slot="drag"
            aria-label={format(m.dragChannel, { channel: channel.name })}
            className="ml-auto rounded p-0.5 text-ink-faint opacity-0 outline-none group-hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <DotsSixVerticalIcon size={14} aria-hidden="true" />
          </Button>
          {channel.ty === "Voice" && (
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

const footerButtonClass =
  "rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";

function UserFooter() {
  const m = useMessages();
  const me = useMe();
  const client = useAspenClient();
  return (
    <div className="flex items-center gap-2 border-t border-line px-3 py-2">
      {me !== null && <Avatar name={displayNameOf(me)} iconId={me.icon} size="sm" />}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-sm font-medium">
          {me === null ? "…" : displayNameOf(me)}
        </span>
        {me?.status != null && (
          <span className="truncate text-xs text-ink-muted">{statusLine(me.status)}</span>
        )}
      </span>
      {me !== null && <EditProfileDialog user={me} triggerClassName={footerButtonClass} />}
      <ThemePicker />
      <Tooltip text={m.signOut}>
        <Button
          aria-label={m.signOut}
          onPress={() => {
            void client.logout();
          }}
          className={footerButtonClass}
        >
          <SignOutIcon size={16} aria-hidden="true" />
        </Button>
      </Tooltip>
    </div>
  );
}
