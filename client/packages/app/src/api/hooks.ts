/**
 * React bindings for the record store. Each hook subscribes to exactly one store topic, so a
 * component re-renders when the record or list it shows changes and not otherwise.
 */

import type {
  AspenSync,
  Attachment,
  Category,
  ChannelVoice,
  Channel,
  Community,
  Icon,
  Invite,
  Message,
  MessageWindow,
  Poll,
  Reactions,
  RecordStore,
  SyncStatus,
  Topic,
  User,
  VoiceCallState,
} from "@aspen/protocol";
import { useCallback, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
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

/** The options the caller has voted for on a poll. */
export function useMyVotes(pollId: string): ReadonlySet<number> {
  return useTopic(`poll:${pollId}`, (s) => s.myVotes(pollId));
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
