/**
 * React bindings for the record store. Each hook subscribes to exactly one store topic, so a
 * component re-renders when the record or list it shows changes and not otherwise.
 */

import type {
  AspenSync,
  Attachment,
  BotCommands,
  Category,
  CategoryOverride,
  Channel,
  ChannelMute,
  ChannelOverride,
  ChannelVoice,
  Community,
  CommunityBan,
  CommunityPermissions,
  CustomEmoji,
  DeploymentPermission,
  Icon,
  Invite,
  KeptMessage,
  LinkedMessage,
  Message,
  MessageWindow,
  NotificationLevel,
  Permission,
  PermissionSet,
  Pin,
  Poll,
  PreferenceDefinition,
  Reactions,
  ReadState,
  RecordStore,
  Role,
  SyncStatus,
  Topic,
  User,
  VoiceCallState,
} from "@aspen/protocol";
import { DEVELOPER_MODE, ID_WIZARD } from "@aspen/protocol";
import { useCallback, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { useBlockedAnywhere } from "./identity";
import { AspenSyncContext } from "./syncContext";

export function useSync(): AspenSync {
  const sync = useContext(AspenSyncContext);
  if (sync === null) {
    throw new Error("useSync must be used inside <SyncProvider>");
  }
  return sync;
}

export function useStore(): RecordStore {
  return useSync().store;
}

export function useSyncStatus(): SyncStatus {
  const sync = useSync();
  return useSyncExternalStore(sync.subscribe, () => sync.status);
}

/** Subscribes to one topic; `read` must return a stable reference until the topic changes. */
function useTopic<T>(topic: Topic, read: (store: RecordStore) => T): T {
  const store = useStore();
  const subscribe = useCallback(
    (listener: () => void) => store.subscribe(topic, listener),
    [store, topic],
  );
  return useSyncExternalStore(subscribe, () => read(store));
}

export function useMe(): User | null {
  return useTopic("me", (s) => s.me());
}

export function useCommunities(): readonly Community[] {
  return useTopic("communities", (s) => s.communities());
}

export function useCommunity(id: string): Community | undefined {
  return useTopic(`community:${id}`, (s) => s.community(id));
}

export function useChannels(communityId: string): readonly Channel[] {
  return useTopic(`channels:${communityId}`, (s) => s.channels(communityId));
}

export function useCategories(communityId: string): readonly Category[] {
  return useTopic(`categories:${communityId}`, (s) => s.categories(communityId));
}

export function useChannel(id: string): Channel | undefined {
  return useTopic(`channel:${id}`, (s) => s.channel(id));
}

/**
 * Several users at once, in the order of `ids`, each `undefined` until it is cached; missing
 * ones are fetched on demand. One subscription per id, one re-render per change.
 */
export function useUsers(ids: readonly string[]): readonly (User | undefined)[] {
  const sync = useSync();
  const store = sync.store;
  const key = ids.join("\n");
  const stableIds = useMemo(() => (key.length === 0 ? [] : key.split("\n")), [key]);
  const cache = useRef<{ key: string; value: (User | undefined)[] } | null>(null);
  const subscribe = useCallback(
    (listener: () => void) => {
      const unsubscribes = stableIds.map((id) =>
        store.subscribe(`user:${id}`, () => {
          cache.current = null;
          listener();
        }),
      );
      return () => {
        for (const unsubscribe of unsubscribes) {
          unsubscribe();
        }
      };
    },
    [store, stableIds],
  );
  const users = useSyncExternalStore(subscribe, () => {
    if (cache.current?.key !== key) {
      cache.current = { key, value: stableIds.map((id) => store.user(id)) };
    }
    return cache.current.value;
  });
  useEffect(() => {
    stableIds.forEach((id, i) => {
      if (users[i] === undefined) {
        sync.ensureUser(id);
      }
    });
  }, [sync, stableIds, users]);
  return users;
}

/** Whether a channel the cache held has since been removed: deleted, or a DM the caller left. */
export function useChannelRemoved(id: string): boolean {
  return useTopic(`channel:${id}`, (s) => s.channelRemoved(id));
}

/** The caller's DMs and group DMs, the most recently active first. */
export function useDms(): readonly Channel[] {
  return useTopic("dms", (s) => s.dms());
}

/** The ids of everyone the caller shares a community with, whom they may start a DM with. */
export function usePeople(): readonly string[] {
  return useTopic("people", (s) => s.people());
}

/** A user by id, fetched on demand when the cache lacks them. `undefined` id reads nothing. */
export function useUser(id: string | undefined): User | undefined {
  const sync = useSync();
  const user = useTopic(`user:${id ?? ""}`, (s) => (id === undefined ? undefined : s.user(id)));
  useEffect(() => {
    if (id !== undefined && user === undefined) {
      sync.ensureUser(id);
    }
  }, [sync, id, user]);
  return user;
}

/**
 * Whether a person's record is still on its way: asked for and neither arrived nor refused.
 * What names them shows a skeleton meanwhile, and "unknown" only once the server says so.
 */
export function useUserLoading(id: string | undefined): boolean {
  return useTopic(`user:${id ?? ""}`, (s) =>
    id === undefined ? false : s.user(id) === undefined && !s.missing("user", id),
  );
}

/** Whether the ID wizard is on: developer mode, and its offer to copy ids, both. */
export function useIdWizard(): boolean {
  const developer = usePreference(DEVELOPER_MODE);
  const wizard = usePreference(ID_WIZARD);
  return developer && wizard;
}

/** An attachment record by id, fetched on demand when the cache lacks it. */
export function useAttachment(id: string): Attachment | undefined {
  const sync = useSync();
  const attachment = useTopic(`attachment:${id}`, (s) => s.attachment(id));
  useEffect(() => {
    if (attachment === undefined) {
      sync.ensureAttachment(id);
    }
  }, [sync, id, attachment]);
  return attachment;
}

/** An icon record by id, fetched on demand when the cache lacks it. `undefined` id reads nothing. */
/**
 * Several icons at once, by id, those the cache lacks fetched; the map is the same object
 * while none of them changes.
 */
export function useIcons(ids: readonly string[]): ReadonlyMap<string, Icon> {
  const store = useStore();
  const sync = useSync();
  const cache = useRef<{ signature: string; icons: ReadonlyMap<string, Icon> }>({
    signature: "",
    icons: new Map(),
  });
  const subscribe = useCallback(
    (listener: () => void) => {
      const unsubscribes = ids.map((id) => store.subscribe(`icon:${id}`, listener));
      return () => {
        for (const unsubscribe of unsubscribes) {
          unsubscribe();
        }
      };
    },
    [store, ids],
  );
  const icons = useSyncExternalStore(subscribe, () => {
    const found = ids.flatMap((id) => {
      const icon = store.icon(id);
      return icon === undefined ? [] : [[id, icon] as const];
    });
    const signature = found.map(([id, icon]) => `${id}=${icon.downloadUrl}`).join("|");
    if (signature !== cache.current.signature) {
      cache.current = { signature, icons: new Map(found) };
    }
    return cache.current.icons;
  });
  useEffect(() => {
    for (const id of ids) {
      if (!icons.has(id)) {
        sync.ensureIcon(id);
      }
    }
  }, [sync, ids, icons]);
  return icons;
}

export function useIcon(id: string | undefined): Icon | undefined {
  const sync = useSync();
  const icon = useTopic(`icon:${id ?? ""}`, (s) => (id === undefined ? undefined : s.icon(id)));
  useEffect(() => {
    if (id !== undefined && icon === undefined) {
      sync.ensureIcon(id);
    }
  }, [sync, id, icon]);
  return icon;
}

/** Whether an icon's record is still on its way: asked for and neither arrived nor refused. */
export function useIconLoading(id: string | undefined): boolean {
  return useTopic(`icon:${id ?? ""}`, (s) =>
    id === undefined ? false : s.icon(id) === undefined && !s.missing("icon", id),
  );
}

/** A poll by id, fetched on demand with the caller's votes when the cache lacks it. */
export function usePoll(id: string): Poll | undefined {
  const sync = useSync();
  const poll = useTopic(`poll:${id}`, (s) => s.poll(id));
  useEffect(() => {
    if (poll === undefined) {
      sync.ensurePoll(id);
    }
  }, [sync, id, poll]);
  return poll;
}

/** Whether a poll is still on its way: asked for and neither arrived nor refused. */
export function usePollLoading(id: string): boolean {
  return useTopic(`poll:${id}`, (s) => s.poll(id) === undefined && !s.missing("poll", id));
}

/** The options the caller has voted for on a poll. */
export function useMyVotes(pollId: string): ReadonlySet<number> {
  return useTopic(`poll:${pollId}`, (s) => s.myVotes(pollId));
}

/** How far the caller has read a channel, if it keeps a read position. */
export function useReadState(channelId: string): ReadState | undefined {
  return useTopic(`read:${channelId}`, (s) => s.readState(channelId));
}

/** Whether the caller may open the Administration Dashboard. */
/** What the caller may do across the deployment. */
export function useDeploymentPermissions(): ReadonlySet<DeploymentPermission> {
  return useTopic("admin", (s) => s.deploymentPermissions());
}

/** Whether the caller may do something across the deployment. */
export function useDeploymentCan(permission: DeploymentPermission): boolean {
  return useDeploymentPermissions().has(permission);
}

/**
 * For a holder of Review reports, how many report cases await review, read when first asked
 * and kept by `reportsChanged` events; `undefined` for anyone else, and until it is read.
 */
export function useOpenReports(): number | undefined {
  const sync = useSync();
  const reviewer = useDeploymentCan("reviewReports");
  const open = useTopic("reports", (s) => s.openReports);
  useEffect(() => {
    if (reviewer && open === undefined) {
      sync.admin.reportCounts().then(
        (counts) => {
          sync.store.setOpenReports(counts.open);
        },
        () => undefined,
      );
    }
  }, [sync, reviewer, open]);
  return reviewer ? open : undefined;
}

/** Counts every change to what awaits review, for a list of reports to read itself again. */
export function useReportsChanges(): number {
  return useTopic("reports", (s) => s.reportsChanges);
}

/** Whether the caller holds any deployment permission, and so has the dashboard to open. */
export function useIsAdmin(): boolean {
  return useDeploymentPermissions().size > 0;
}

/** Whether the caller has a category collapsed in their channel list. */
export function useCollapsed(categoryId: string): boolean {
  return useTopic(`collapse:${categoryId}`, (s) => s.collapsed(categoryId));
}

/**
 * The channels among `channelIds` that stay in view under a collapsed category: the unread and
 * unmuted ones, and voice channels with someone in the call. Subscribes to `unread`, which
 * every read state and mute change touches, and to each channel's call.
 */
export function useShownWhenCollapsed(channelIds: readonly string[]): ReadonlySet<string> {
  const store = useStore();
  const key = channelIds.join("\n");
  const subscribe = useCallback(
    (listener: () => void) => {
      const ids = key === "" ? [] : key.split("\n");
      const unsubscribes = ["unread", ...ids.map((id) => `voice:${id}`)].map((topic) =>
        store.subscribe(topic, listener),
      );
      return () => {
        for (const unsubscribe of unsubscribes) {
          unsubscribe();
        }
      };
    },
    [store, key],
  );
  // A string, so the snapshot compares equal while nothing changes.
  const shown = useSyncExternalStore(subscribe, () =>
    (key === "" ? [] : key.split("\n")).filter((id) => store.shownWhenCollapsed(id)).join("\n"),
  );
  return useMemo(() => new Set(shown === "" ? [] : shown.split("\n")), [shown]);
}

/** Whether the caller has blocked `userId`. */
/**
 * Whether the caller blocked `userId`, here or on any other deployment they use
 * (`useBlockedAnywhere`), as the client hides a blocked person everywhere.
 */
export function useBlocked(userId: string | undefined): boolean {
  const here = useTopic(`block:${userId ?? ""}`, (s) => userId !== undefined && s.blocked(userId));
  const elsewhere = useBlockedAnywhere(userId);
  return here || elsewhere;
}

/**
 * The other person of a one-to-one DM (or of the DM a thread is in) whom the caller blocked,
 * so that nothing may be written there; `null` otherwise.
 */
export function useBlockedDmPeer(channelId: string): string | null {
  return useTopic(`channelAccess:${channelId}`, (s) => s.blockedDmPeer(channelId));
}

/** The system account, when it is the other person of this DM, whose notices are only read. */
export function useSystemDmPeer(channelId: string): string | null {
  return useTopic(`channelAccess:${channelId}`, (s) => s.systemDmPeer(channelId));
}

/** The bots the caller owns, as far as the cache holds them (`AspenSync.loadBots`). */
export function useOwnedBots(): readonly User[] {
  return useTopic("bots", (s) => s.ownedBots());
}

/** Everyone the caller has blocked. */
export function useBlockedUsers(): readonly string[] {
  return useTopic("blocks", (s) => s.blockedUsers());
}

/**
 * Which of `userIds` are silenced and hidden in calls here: blocked here or, as the same
 * person, on any other deployment the viewer uses.
 */
export function useSilenced(userIds: readonly string[]): ReadonlySet<string> {
  const key = useTopic("silenced", (s) => userIds.filter((id) => s.silenced(id)).join(","));
  return useMemo(() => new Set(key === "" ? [] : key.split(",")), [key]);
}

/** The caller's mute of a channel while it lasts, if any. */
export function useMute(channelId: string): ChannelMute | undefined {
  return useTopic(`mute:${channelId}`, (s) => s.mute(channelId));
}

/**
 * How much of a channel the caller is told of: the level in force, the channel's own setting if
 * it has one, and what it would be without it (`RecordStore.notificationLevel`).
 */
export function useNotificationLevel(channelId: string): {
  level: NotificationLevel;
  own: NotificationLevel | null;
  inherited: NotificationLevel;
} {
  const level = useTopic("notifications", (s) => s.notificationLevel(channelId).level);
  const own = useTopic("notifications", (s) => s.notificationLevel(channelId).own);
  const inherited = useTopic("notifications", (s) => s.notificationLevel(channelId).inherited);
  return { level, own, inherited };
}

/** The caller's setting for a community, if they made one. */
export function useCommunityNotificationLevel(communityId: string): NotificationLevel | null {
  return useTopic("notifications", (s) => s.communityNotificationLevel(communityId));
}

/** How many unread messages in a channel tag the caller. */
export function useMentions(channelId: string): number {
  return useTopic(`read:${channelId}`, (s) => s.mentions(channelId));
}

/** How many unread messages tag the caller across a community, or their DMs (`UNREAD_DMS`). */
export function usePlaceMentions(place: string): number {
  return useTopic("unread", (s) => s.placeMentions(place));
}

/** Whether a channel holds a message by someone else that the caller has not read. */
export function useUnread(channelId: string): boolean {
  return useTopic(`read:${channelId}`, (s) => s.unread(channelId));
}

/** The communities with an unread channel, and `UNREAD_DMS` when a DM is unread. */
export function useUnreadPlaces(): ReadonlySet<string> {
  return useTopic("unread", (s) => s.unreadPlaces());
}

/** The options on a poll that are the caller's own write-ins. */
export function useMyWriteIns(pollId: string): ReadonlySet<number> {
  return useTopic(`poll:${pollId}`, (s) => s.myWriteIns(pollId));
}

/**
 * Several attachment records at once, in the order of `ids`, each `undefined` until it is
 * cached; missing ones are fetched on demand. One subscription per id, one re-render per change.
 */
export function useAttachments(ids: readonly string[]): readonly (Attachment | undefined)[] {
  const sync = useSync();
  const store = sync.store;
  const key = ids.join("\n");
  const stableIds = useMemo(() => (key.length === 0 ? [] : key.split("\n")), [key]);
  const cache = useRef<{ key: string; value: (Attachment | undefined)[] } | null>(null);
  const subscribe = useCallback(
    (listener: () => void) => {
      const unsubscribes = stableIds.map((id) =>
        store.subscribe(`attachment:${id}`, () => {
          cache.current = null;
          listener();
        }),
      );
      return () => {
        for (const unsubscribe of unsubscribes) {
          unsubscribe();
        }
      };
    },
    [store, stableIds],
  );
  const records = useSyncExternalStore(subscribe, () => {
    if (cache.current?.key !== key) {
      cache.current = { key, value: stableIds.map((id) => store.attachment(id)) };
    }
    return cache.current.value;
  });
  useEffect(() => {
    stableIds.forEach((id, i) => {
      if (records[i] === undefined) {
        sync.ensureAttachment(id);
      }
    });
  }, [sync, stableIds, records]);
  return records;
}

/** The call on a voice channel and who is in it. */
export function useChannelVoice(channelId: string): ChannelVoice {
  return useTopic(`voice:${channelId}`, (s) => s.channelVoice(channelId));
}

/** One of the user's preferences, current as it changes here or on another device. */
export function usePreference<T>(definition: PreferenceDefinition<T>): T {
  const sync = useSync();
  const subscribe = useCallback(
    (listener: () => void) => sync.preferences.subscribe(listener),
    [sync],
  );
  return useSyncExternalStore(subscribe, () => sync.preferences.get(definition));
}

/** The user's own call, idle or not. */
export function useVoiceCall(): VoiceCallState {
  const sync = useSync();
  return useSyncExternalStore(sync.voice.subscribe, () => sync.voice.state);
}

export function useMessageWindow(channelId: string): MessageWindow | undefined {
  return useTopic(`messages:${channelId}`, (s) => s.messages(channelId));
}

export function useMessage(id: string): Message | undefined {
  return useTopic(`message:${id}`, (s) => s.message(id));
}

/** What the caller finds at a message another links to, once a read has said. */
export function useLinkedMessage(id: string): LinkedMessage | undefined {
  return useTopic(`link:${id}`, (s) => s.linkedMessage(id));
}

/** A message a warning is about, deleted or not, once a read of the warning has brought it. */
export function useWarnedMessage(id: string): KeptMessage | undefined {
  return useTopic(`warned:${id}`, (s) => s.warnedMessage(id));
}

/** A community's member records; re-renders when membership or any member's status changes. */
export function useMembers(communityId: string): readonly User[] {
  return useTopic(`members:${communityId}`, (s) => s.members(communityId));
}

export function useInvites(communityId: string): readonly Invite[] {
  return useTopic(`invites:${communityId}`, (s) => s.invites(communityId));
}

export function useReactions(messageId: string): Reactions {
  return useTopic(`reactions:${messageId}`, (s) => s.reactions(messageId));
}

/** A community's roles, lowest first. */
export function useRoles(communityId: string): readonly Role[] {
  return useTopic(`roles:${communityId}`, (s) => s.roles(communityId));
}

/** A community's own emoji, by name. */
export function useCustomEmoji(communityId: string): readonly CustomEmoji[] {
  return useTopic(`emoji:${communityId}`, (s) => s.customEmoji(communityId));
}

/**
 * A community's standing bans, newest first, read on first use for a holder of Ban members;
 * `undefined` until read.
 */
export function useBans(communityId: string): readonly CommunityBan[] | undefined {
  const sync = useSync();
  const bans = useTopic(`bans:${communityId}`, (s) => s.bans(communityId));
  useEffect(() => {
    if (bans === undefined) {
      void sync.loadBans(communityId).catch(() => undefined);
    }
  }, [sync, communityId, bans]);
  return bans;
}

/** The roles a member holds besides everyone's, or `undefined` while unknown. */
export function useMemberRoles(communityId: string, userId: string): readonly string[] | undefined {
  return useTopic(`roles:${communityId}`, (s) => s.memberRoles(communityId, userId));
}

/** What the caller may do across a community; `null` while that is unknown. */
export function useAccess(communityId: string): CommunityPermissions | null {
  return useTopic(`access:${communityId}`, (s) => s.access(communityId));
}

/** Whether the caller holds a community permission. */
export function useCan(communityId: string | null | undefined, permission: Permission): boolean {
  const access = useTopic(`access:${communityId ?? ""}`, (s) =>
    communityId == null ? null : s.access(communityId),
  );
  return access?.has(permission) ?? false;
}

/** What the caller may do in a channel (a thread's are its parent's; a DM's are every one). */
export function useChannelAccess(channelId: string): PermissionSet {
  return useTopic(`channelAccess:${channelId}`, (s) => s.channelAccess(channelId));
}

/** Whether the caller may do something in a channel. */
export function useChannelCan(channelId: string, permission: Permission): boolean {
  return useChannelAccess(channelId).has(permission);
}

export function useChannelOverrides(channelId: string): readonly ChannelOverride[] {
  return useTopic(`overrides:${channelId}`, (s) => s.channelOverrides(channelId));
}

export function useCategoryOverrides(categoryId: string): readonly CategoryOverride[] {
  return useTopic(`overrides:${categoryId}`, (s) => s.categoryOverrides(categoryId));
}

/** A channel's pins in their order, `undefined` until read; the first use reads them. */
export function usePins(channelId: string): readonly Pin[] | undefined {
  const sync = useSync();
  const pins = useTopic(`pins:${channelId}`, (s) => s.pins(channelId));
  useEffect(() => {
    if (pins === undefined) {
      void sync.loadPins(channelId).catch(() => undefined);
    }
  }, [sync, channelId, pins]);
  return pins;
}

/**
 * The commands of the bots that can see a channel, read when first asked for and again
 * whenever the store drops them; `undefined` while they are read, or for no channel.
 */
export function useCommands(channelId: string | null): readonly BotCommands[] | undefined {
  const sync = useSync();
  const topic = channelId === null ? "commands:" : `commands:${channelId}`;
  const commands = useTopic(topic, (s) => (channelId === null ? undefined : s.commands(channelId)));
  useEffect(() => {
    if (channelId !== null && commands === undefined) {
      void sync.loadCommands(channelId).catch(() => undefined);
    }
  }, [sync, channelId, commands]);
  return commands;
}
