/**
 * Keeps a `RecordStore` in step with one Aspen server.
 *
 * The server keeps sixty seconds of events replayable, and a fresh event-stream connection
 * replays that whole window. `AspenSync` relies on it: it bootstraps the cache from REST first
 * and connects the stream second, so every change committed during the bootstrap is replayed
 * on top of it. Events are JSON Merge Patches and deletes are idempotent, so replaying a change
 * the bootstrap already reflected is harmless.
 *
 * The one ordering hazard is a resync: the stream is already open when the server reports it
 * could not resume, so events that arrive while the cache is being re-read are held and applied
 * after the read lands. Otherwise a slow REST response could overwrite a newer event. The same
 * hold covers the first bootstrap, which makes both paths one piece of code.
 */

import { EventStream, type EventStreamOptions } from "./events";
import type { ServerEvent } from "./generated/events";
import type { components } from "./generated/openapi";
import { type AspenClient, problemOf } from "./http";
import { ApiProblemError, type Problem, transportProblem } from "./problem";
import {
  AUDIO_INPUT,
  AUDIO_OUTPUT,
  PreferenceStore,
  effectiveUserVolume,
  userMuted,
  userVolume,
  type PreferenceStorage,
} from "./preferences";
import { RecordStore } from "./store";
import { eventStreamUrl } from "./urls";
import { VoiceCall, type VoiceMedia } from "./voice";

type Message = components["schemas"]["Message"];
type Attachment = components["schemas"]["Attachment"];
type Invite = components["schemas"]["Invite"];
type Community = components["schemas"]["Community"];
type Channel = components["schemas"]["Channel"];
type ChannelType = components["schemas"]["ChannelType"];
type Category = components["schemas"]["Category"];
type Poll = components["schemas"]["Poll"];
type PollCreateRequest = components["schemas"]["PollCreateRequest"];
type User = components["schemas"]["User"];
type Icon = components["schemas"]["Icon"];
type CommunityUpdateRequest = components["schemas"]["CommunityUpdateRequest"];
type UserUpdateRequest = components["schemas"]["UserUpdateRequest"];

export interface InviteLookup {
  invite: Invite;
  community: Community;
  /** Whether the invite's `expiresAt` is in the past. */
  expired: boolean;
  /** Whether the caller already belongs to the community. */
  member: boolean;
}

/** Messages fetched per page, for the latest window and each older page. */
export const MESSAGE_PAGE_SIZE = 50;
/** Messages fetched on each side of a linked message. */
export const MESSAGE_AROUND_RADIUS = 25;
/** How long the server keeps events replayable; mirrors the server's `MAX_EVENT_AGE`. */
export const EVENT_REPLAY_WINDOW_MS = 60_000;
/**
 * A bootstrap that took longer than this before the stream connected may have missed events
 * that fell out of the replay window, so it is redone once the stream is up.
 */
const BOOTSTRAP_STALE_AFTER_MS = EVENT_REPLAY_WINDOW_MS - 10_000;
/**
 * How often presence is asked for while the sync is live and the page is visible. Presence is
 * pulled for the users on screen rather than pushed to everyone.
 */
export const PRESENCE_POLL_MS = 30_000;
/**
 * The least time between two activity reports; mirrors the event stream's `ACTIVITY_INTERVAL`,
 * which ignores reports closer together.
 */
export const ACTIVITY_INTERVAL_MS = 60_000;

/** The most users one presence request names; mirrors the server's limit. */
export const PRESENCE_BATCH = 100;

export type SyncStatus =
  /** `start()` has not been called, or `stop()` has. */
  | "stopped"
  /** Reading the initial state from REST; nothing is cached yet. */
  | "bootstrapping"
  /** The cache is loaded and usable; the event stream has not connected yet. */
  | "connecting"
  /** Cache is current and the event stream is connected. */
  | "live"
  /** The stream dropped; the cache is current as of the last event and reconnection is under way. */
  | "reconnecting"
  /** The stream is back but could not replay the gap; the cache is being re-read. */
  | "resyncing"
  /** A bootstrap failed. `lastError` says why; `start()` tries again. */
  | "failed";

export interface AspenSyncOptions {
  client: AspenClient;
  /** Supply one to share a store across sync instances; a new one is created otherwise. */
  store?: RecordStore;
  /** Validate incoming event frames against the schema. Off by default. */
  validateEvents?: boolean;
  /** Called with a frame that failed validation and was dropped. Only fires when validating. */
  onInvalidEvent?: (raw: unknown, errors: string) => void;
  /** Overrides for tests and non-browser shells. */
  WebSocket?: typeof globalThis.WebSocket;
  setTimeout?: typeof globalThis.setTimeout;
  clearTimeout?: typeof globalThis.clearTimeout;
  now?: () => number;
  /**
   * Used to `PUT` attachment bytes to the presigned storage URL. That request goes to the
   * object store, not the API, and must not carry the session token. Defaults to the global
   * `fetch`.
   */
  uploadFetch?: typeof globalThis.fetch;
  /** The browser's media for voice calls; `browserVoiceMedia()` in the app, a fake in tests. */
  voiceMedia?: VoiceMedia;
  /**
   * Where device preferences live. Defaults to the page's `localStorage` when there is one;
   * `null` keeps them for the session only.
   */
  preferenceStorage?: PreferenceStorage | null;
  /** Uniform in [0, 1); seeds the voice rejoin delay. */
  random?: () => number;
}

export type SyncListener = () => void;

export class AspenSync {
  readonly store: RecordStore;
  /** The voice call, if any; a `VoiceCall` even when idle so the UI can subscribe once. */
  readonly voice: VoiceCall;
  /** The user's preferences, device-scoped and account-scoped alike. */
  readonly preferences: PreferenceStore;
  readonly #client: AspenClient;
  readonly #stream: EventStream;
  readonly #now: () => number;
  readonly #uploadFetch: typeof globalThis.fetch;
  readonly #listeners = new Set<SyncListener>();
  #status: SyncStatus = "stopped";
  #lastError: Problem | null = null;
  /** Events received while a bootstrap is in flight, applied once it lands. `null` when live. */
  #held: ServerEvent[] | null = null;
  #bootstrappedAt = 0;
  /** Increments on every start/stop so a stale async step can notice and bail. */
  #generation = 0;
  readonly #windowLoads = new Map<string, Promise<void>>();
  readonly #userLoads = new Map<string, Promise<void>>();
  readonly #setTimeout: typeof globalThis.setTimeout;
  #presenceTimer: ReturnType<typeof setTimeout> | null = null;
  /** When the user last did something in the app, and when the server was last told. */
  #lastActivityAt = Number.NEGATIVE_INFINITY;
  #activityReportedAt = Number.NEGATIVE_INFINITY;
  /** Users the server said do not exist; asked once, not again. */
  readonly #missingUsers = new Set<string>();
  readonly #attachmentLoads = new Map<string, Promise<void>>();
  readonly #missingAttachments = new Set<string>();
  readonly #pollLoads = new Map<string, Promise<void>>();
  readonly #missingPolls = new Set<string>();
  readonly #iconLoads = new Map<string, Promise<void>>();
  readonly #missingIcons = new Set<string>();

  constructor(options: AspenSyncOptions) {
    this.#client = options.client;
    this.store = options.store ?? new RecordStore({ now: options.now ?? (() => Date.now()) });
    this.#now = options.now ?? (() => Date.now());
    this.#uploadFetch = options.uploadFetch ?? ((input, init) => globalThis.fetch(input, init));
    const voiceOptions: ConstructorParameters<typeof VoiceCall>[0] = {
      client: options.client,
      media: options.voiceMedia ?? lazyBrowserMedia(),
      now: this.#now,
    };
    if (options.WebSocket !== undefined) {
      voiceOptions.WebSocket = options.WebSocket;
    }
    if (options.setTimeout !== undefined) {
      voiceOptions.setTimeout = options.setTimeout;
    }
    if (options.random !== undefined) {
      voiceOptions.random = options.random;
    }
    voiceOptions.userVolume = (userId) => effectiveUserVolume(this.preferences, userId);
    this.voice = new VoiceCall(voiceOptions);
    this.#setTimeout = options.setTimeout ?? globalThis.setTimeout.bind(globalThis);
    if (typeof document !== "undefined") {
      // A page coming back into view gets fresh presence at once rather than at the next tick.
      document.addEventListener("visibilitychange", () => {
        if (document.visibilityState === "visible" && this.#status === "live") {
          void this.#pollPresence();
        }
      });
    }
    this.preferences = new PreferenceStore({
      storage: options.preferenceStorage === undefined ? pageStorage() : options.preferenceStorage,
      client: this.#client,
    });
    // The devices voice chat uses follow the preferences, now and whenever they change.
    const applyDevices = () => {
      void this.voice
        .setAudioDevices({
          input: this.preferences.get(AUDIO_INPUT),
          output: this.preferences.get(AUDIO_OUTPUT),
        })
        .catch(() => undefined);
    };
    this.preferences.subscribe(applyDevices);
    applyDevices();
    const streamOptions: EventStreamOptions = {
      url: eventStreamUrl(options.client.baseUrl),
      authenticate: (o) => options.client.freshSessionToken(o),
      onReady: (info) => {
        this.#onReady(info.resumed);
      },
      onEvent: (event) => {
        this.#onEvent(event);
      },
      onResyncRequired: () => {
        void this.#resync();
      },
      onConnectionLost: () => {
        if (this.#status === "live" || this.#status === "connecting") {
          this.#setStatus("reconnecting");
        }
      },
    };
    if (options.validateEvents !== undefined) {
      streamOptions.validate = options.validateEvents;
    }
    if (options.onInvalidEvent !== undefined) {
      streamOptions.onInvalidEvent = options.onInvalidEvent;
    }
    if (options.WebSocket !== undefined) {
      streamOptions.WebSocket = options.WebSocket;
    }
    if (options.setTimeout !== undefined) {
      streamOptions.setTimeout = options.setTimeout;
    }
    if (options.clearTimeout !== undefined) {
      streamOptions.clearTimeout = options.clearTimeout;
    }
    this.#stream = new EventStream(streamOptions);
  }

  get status(): SyncStatus {
    return this.#status;
  }

  get lastError(): Problem | null {
    return this.#lastError;
  }

  /** Registers for status changes and returns the unsubscribe function. */
  readonly subscribe = (listener: SyncListener): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  /** Bootstraps the cache and then connects the event stream. Safe to call again after `failed`. */
  start(): void {
    if (this.#status !== "stopped" && this.#status !== "failed") {
      return;
    }
    this.#generation += 1;
    const generation = this.#generation;
    this.#lastError = null;
    this.#setStatus("bootstrapping");
    void this.#bootstrap(generation).then((ok) => {
      if (ok && generation === this.#generation) {
        this.#setStatus("connecting");
        this.#stream.start();
      }
    });
  }

  /** Disconnects and forgets the cache. */
  stop(): void {
    this.#generation += 1;
    this.#stream.stop();
    if (this.#presenceTimer !== null) {
      clearTimeout(this.#presenceTimer);
      this.#presenceTimer = null;
    }
    this.voice.leave();
    this.preferences.clearAccount();
    this.#held = null;
    this.#windowLoads.clear();
    this.#userLoads.clear();
    this.#pollLoads.clear();
    this.#iconLoads.clear();
    this.store.clear();
    this.#setStatus("stopped");
  }

  /** Loads the newest page of a channel, replacing whatever window is loaded. */
  loadLatest(channelId: string): Promise<void> {
    return this.#loadWindow(channelId, async () => {
      const messages = await this.#readMessages(channelId, { limit: MESSAGE_PAGE_SIZE });
      this.store.replaceWindow(channelId, messages, {
        hasOlder: messages.length === MESSAGE_PAGE_SIZE,
        atLatest: true,
      });
    });
  }

  /** Extends the loaded window backwards by one page. No-op without a window or older messages. */
  loadOlder(channelId: string): Promise<void> {
    return this.#loadWindow(channelId, async () => {
      const window = this.store.messages(channelId);
      const oldest = window?.ids[0];
      if (window === undefined || oldest === undefined || !window.hasOlder) {
        return;
      }
      const messages = await this.#readMessages(channelId, {
        before: oldest,
        limit: MESSAGE_PAGE_SIZE,
      });
      this.store.prependWindow(channelId, messages, messages.length === MESSAGE_PAGE_SIZE);
    });
  }

  /**
   * Extends the loaded window forwards by one page, for a window that is not at the latest.
   * The window is at the latest once a read comes back short.
   */
  loadNewer(channelId: string): Promise<void> {
    return this.#loadWindow(channelId, async () => {
      const window = this.store.messages(channelId);
      const newest = window?.ids[window.ids.length - 1];
      if (window === undefined || newest === undefined || window.atLatest) {
        return;
      }
      const messages = await this.#readMessages(channelId, {
        after: newest,
        limit: MESSAGE_PAGE_SIZE,
      });
      this.store.appendWindow(channelId, messages, messages.length < MESSAGE_PAGE_SIZE);
    });
  }

  /**
   * Loads the window around one message, for a link to it. The window is not at the latest
   * unless the read shows nothing newer, so new messages are not appended to it; `loadLatest`
   * brings the channel back to the present.
   */
  loadAround(channelId: string, messageId: string): Promise<void> {
    return this.#loadWindow(channelId, async () => {
      const messages = await this.#readMessages(channelId, {
        around: messageId,
        limit: MESSAGE_AROUND_RADIUS,
      });
      let older = 0;
      let newer = 0;
      for (const message of messages) {
        if (message.id < messageId) {
          older += 1;
        } else if (message.id > messageId) {
          newer += 1;
        }
      }
      this.store.replaceWindow(channelId, messages, {
        hasOlder: older >= MESSAGE_AROUND_RADIUS,
        atLatest: newer < MESSAGE_AROUND_RADIUS,
      });
    });
  }

  /**
   * Uploads a file in the server's two phases, reserving an attachment, sending the bytes
   * straight to storage, and confirming, and caches the resulting record. The attachment can
   * then be named in a message.
   */
  async uploadAttachment(file: File): Promise<Attachment> {
    const init = await this.#client.api.POST("/api/v1/attachments", {
      body: { fileName: file.name, mimeType: file.type || "application/octet-stream" },
    });
    if (init.data === undefined) {
      throw new ApiProblemError(problemOf(init.error, init.response));
    }
    const put = await this.#uploadFetch(init.data.uploadUrl, {
      method: "PUT",
      headers: { "content-type": file.type || "application/octet-stream" },
      body: file,
    });
    if (!put.ok) {
      throw new ApiProblemError(
        transportProblem(`upload failed: ${String(put.status)} ${put.statusText}`, put.status),
      );
    }
    const confirm = await this.#client.api.POST("/api/v1/attachments/{attachment}/confirm", {
      params: { path: { attachment: init.data.id } },
    });
    if (confirm.data === undefined) {
      throw new ApiProblemError(problemOf(confirm.error, confirm.response));
    }
    this.store.ingest({ attachments: [confirm.data] });
    return confirm.data;
  }

  /**
   * Uploads an icon in the server's two phases, reserving it, sending the bytes straight to
   * storage, and confirming, and caches the record. The caller then names the icon on a user
   * or community.
   */
  async uploadIcon(bytes: Blob, mimeType: string): Promise<Icon> {
    const init = await this.#client.api.POST("/api/v1/icons", { body: { mimeType } });
    if (init.data === undefined) {
      throw new ApiProblemError(problemOf(init.error, init.response));
    }
    const put = await this.#uploadFetch(init.data.uploadUrl, {
      method: "PUT",
      headers: { "content-type": mimeType },
      body: bytes,
    });
    if (!put.ok) {
      throw new ApiProblemError(
        transportProblem(`upload failed: ${String(put.status)} ${put.statusText}`, put.status),
      );
    }
    const confirm = await this.#client.api.POST("/api/v1/icons/{icon}/confirm", {
      params: { path: { icon: init.data.id } },
    });
    if (confirm.data === undefined) {
      throw new ApiProblemError(problemOf(confirm.error, confirm.response));
    }
    this.store.putIcon(confirm.data);
    return confirm.data;
  }

  /** Fetches an icon record the cache lacks, once, for a user or community that names it. */
  ensureIcon(iconId: string): void {
    if (
      this.store.icon(iconId) !== undefined ||
      this.#missingIcons.has(iconId) ||
      this.#iconLoads.has(iconId)
    ) {
      return;
    }
    const load = this.#client.api
      .GET("/api/v1/icons/{icon}", { params: { path: { icon: iconId } } })
      .then(({ data, response }) => {
        if (data !== undefined) {
          this.store.putIcon(data);
        } else if (response.status === 404) {
          this.#missingIcons.add(iconId);
        }
      })
      .catch(() => {
        // Transient; the next render that needs the icon asks again.
      })
      .finally(() => {
        this.#iconLoads.delete(iconId);
      });
    this.#iconLoads.set(iconId, load);
  }

  /**
   * Arranges the caller's communities in the given order. Every community whose position
   * changed is renumbered to its position and patched; the cache is arranged at once, so the
   * membership update events that follow are no-ops.
   */
  async reorderCommunities(orderedIds: readonly string[]): Promise<void> {
    const changed = orderedIds.flatMap((id, index) =>
      this.store.communityOrder(id) === index ? [] : [{ id, index }],
    );
    for (const { id, index } of changed) {
      this.store.setCommunityOrder(id, index);
    }
    await Promise.all(
      changed.map(async ({ id, index }) => {
        const result = await this.#client.api.PATCH("/api/v1/communities/{community}/members/@me", {
          params: { path: { community: id } },
          body: { sortIndex: index },
        });
        if (result.error !== undefined) {
          throw new ApiProblemError(problemOf(result.error, result.response));
        }
      }),
    );
  }

  /**
   * Arranges the channels of one group, a category's (`parentCategory` its id) or a
   * community's top-level ones (`null`), in the given order. A channel in the order that
   * belongs elsewhere is moved into the group. Positions are the sort indexes, so groups
   * number independently and only the order within a group means anything. Every channel
   * whose index or category changed is patched; the cache is arranged at once, so the channel
   * update events that follow are no-ops.
   */
  async arrangeChannels(
    orderedIds: readonly string[],
    parentCategory: string | null,
  ): Promise<void> {
    const changed = orderedIds.flatMap((id, index) => {
      const channel = this.store.channel(id);
      if (channel === undefined) {
        return [];
      }
      const moved = (channel.parentCategory ?? null) !== parentCategory;
      return channel.sortIndex === index && !moved ? [] : [{ id, index, moved }];
    });
    for (const { id, index, moved } of changed) {
      this.store.applyEvent({
        serverEvent: "channel",
        type: "update",
        id,
        sortIndex: index,
        ...(moved ? { parentCategory } : {}),
      });
    }
    await Promise.all(
      changed.map(async ({ id, index, moved }) => {
        const result = await this.#client.api.PATCH("/api/v1/channels/{channel}", {
          params: { path: { channel: id } },
          body: { sortIndex: index, ...(moved ? { parentCategory } : {}) },
        });
        if (result.error !== undefined) {
          throw new ApiProblemError(problemOf(result.error, result.response));
        }
      }),
    );
  }

  /**
   * Changes a community's name or icon as a merge patch. The cache is left to the update
   * event, which the server publishes before it answers.
   */
  async updateCommunity(communityId: string, patch: CommunityUpdateRequest): Promise<Community> {
    const result = await this.#client.api.PATCH("/api/v1/communities/{community}", {
      params: { path: { community: communityId } },
      body: patch,
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    return result.data;
  }

  /** Fetches an attachment record the cache lacks, once, for a message that only names it. */
  ensureAttachment(attachmentId: string): void {
    if (
      this.store.attachment(attachmentId) !== undefined ||
      this.#missingAttachments.has(attachmentId) ||
      this.#attachmentLoads.has(attachmentId)
    ) {
      return;
    }
    const load = this.#client.api
      .GET("/api/v1/attachments/{attachment}", { params: { path: { attachment: attachmentId } } })
      .then(({ data, response }) => {
        if (data !== undefined) {
          this.store.ingest({ attachments: [data] });
        } else if (response.status === 404) {
          this.#missingAttachments.add(attachmentId);
        }
      })
      .catch(() => {
        // Transient; the next render that needs the attachment asks again.
      })
      .finally(() => {
        this.#attachmentLoads.delete(attachmentId);
      });
    this.#attachmentLoads.set(attachmentId, load);
  }

  /**
   * Posts a message, optionally naming uploaded attachments. In a thread, `echoToParent` also
   * shows it in the thread's parent channel. The result is cached at once unless the stream
   * delivered the message first, in which case the streamed copy is newer and is kept.
   */
  async sendMessage(
    channelId: string,
    content: string,
    attachments: readonly string[] = [],
    options: { echoToParent?: boolean } = {},
  ): Promise<Message> {
    const result = await this.#client.api.POST("/api/v1/channels/{channel}/messages", {
      params: { path: { channel: channelId } },
      body: {
        content,
        attachments: [...attachments],
        ...(options.echoToParent === true ? { echoToParent: true } : {}),
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.addMessage(result.data);
    return result.data;
  }

  /**
   * Opens the thread a message started, which the server makes the first time. The thread is
   * cached at once and the message learns its thread, as the events that follow would say.
   */
  async openThread(messageId: string): Promise<Channel> {
    const result = await this.#client.api.PUT("/api/v1/messages/{message}/thread", {
      params: { path: { message: messageId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const thread = result.data;
    this.store.ingest({ channels: [thread] });
    this.store.applyEvent({
      serverEvent: "message",
      type: "update",
      id: messageId,
      thread: thread.id,
    });
    return thread;
  }

  /**
   * Reads one channel into the store when it is not there yet: a thread opened from a link, or
   * a DM from before the last listing. Resolves to the channel, or rejects when it is gone or
   * not the caller's to see.
   */
  async loadChannel(channelId: string): Promise<Channel> {
    const held = this.store.channel(channelId);
    if (held !== undefined) {
      return held;
    }
    const result = await this.#client.api.GET("/api/v1/channels/{channel}", {
      params: { path: { channel: channelId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ channels: [result.data] });
    return result.data;
  }

  /**
   * Reads one message into the store when it is not there yet, such as the message a thread
   * opened from a link started. Its author, attachments, poll, and thread come with it.
   */
  async loadMessage(messageId: string): Promise<Message> {
    const held = this.store.message(messageId);
    if (held !== undefined) {
      return held;
    }
    const result = await this.#client.api.GET("/api/v1/messages/{message}", {
      params: {
        path: { message: messageId },
        query: { include: ["authors", "attachments", "polls", "threads"] },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ ...result.data.included, messages: [result.data.data] });
    return result.data.data;
  }

  /**
   * Opens a DM with the people named: with one person their one-to-one DM, which the server
   * returns as it is when it exists; with more, a new group DM.
   */
  async openDm(recipients: readonly string[]): Promise<Channel> {
    const result = await this.#client.api.POST("/api/v1/users/@me/dms", {
      body: { recipients: [...recipients] },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ channels: [result.data] });
    return result.data;
  }

  /** Adds someone to a group DM; the change arrives as the DM's event. */
  async addDmRecipient(channelId: string, userId: string): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/channels/{channel}/recipients/{user}", {
      params: { path: { channel: channelId, user: userId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Leaves a group DM. The DM's event, which names its recipients without the caller, is what
   * takes it out of the store.
   */
  async leaveDm(channelId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/channels/{channel}/recipients/@me", {
      params: { path: { channel: channelId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Creates a community, which the server also joins the caller to and gives a default text and
   * voice channel. The server publishes no event for the community itself, so the response is
   * cached here and the community is then read whole for its channels.
   */
  async createCommunity(name: string): Promise<Community> {
    const result = await this.#client.api.POST("/api/v1/communities", {
      body: { name, icon: null },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const community = result.data;
    const me = this.store.myUserId;
    this.store.ingest({ communities: [community] });
    if (me !== null) {
      this.store.applyEvent({
        serverEvent: "userCommunity",
        type: "create",
        community: community.id,
        user: me,
        sortIndex: this.store.communities().length,
      });
    }
    await this.loadCommunity(community.id);
    return community;
  }

  /**
   * Creates a channel at the end of its community's sort order, filed under `parentCategory`
   * when given. The result is cached at once; the matching event is then a no-op.
   */
  async createChannel(
    communityId: string,
    options: { name: string; ty: ChannelType; parentCategory: string | null },
  ): Promise<Channel> {
    const sortIndex = this.store
      .channels(communityId)
      .reduce((max, channel) => Math.max(max, channel.sortIndex + 1), 0);
    const result = await this.#client.api.POST("/api/v1/channels", {
      body: {
        name: options.name,
        ty: options.ty,
        community: communityId,
        parentCategory: options.parentCategory,
        sortIndex,
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ channels: [result.data] });
    return result.data;
  }

  /**
   * Replaces a message's text. The cache is left to the update event, which the server
   * publishes before it answers: writing the response here could clobber a newer event that
   * arrived while the request was in flight, such as the link previews refetched for the new
   * text.
   */
  async editMessage(messageId: string, content: string): Promise<Message> {
    const result = await this.#client.api.PATCH("/api/v1/messages/{message}", {
      params: { path: { message: messageId } },
      body: { content },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    return result.data;
  }

  /**
   * Adds the caller's reaction. The cache is updated at once so the chip responds without
   * waiting for the event; the event itself is then a no-op.
   */
  async addReaction(messageId: string, emoji: string): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/messages/{message}/reactions/{emoji}/@me", {
      params: { path: { message: messageId, emoji } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({ serverEvent: "react", type: "create", ...result.data });
  }

  /** Removes the caller's reaction, dropping it from the cache at once. */
  async removeReaction(messageId: string, emoji: string): Promise<void> {
    const me = this.store.myUserId;
    const result = await this.#client.api.DELETE(
      "/api/v1/messages/{message}/reactions/{emoji}/@me",
      { params: { path: { message: messageId, emoji } } },
    );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    if (me !== null) {
      this.store.applyEvent({
        serverEvent: "react",
        type: "delete",
        messageId,
        emoji,
        userId: me,
      });
    }
  }

  /**
   * Opens a poll in a channel. The poll is cached at once; the message that shows it arrives
   * on the stream.
   */
  async createPoll(channelId: string, request: PollCreateRequest): Promise<Poll> {
    const result = await this.#client.api.POST("/api/v1/channels/{channel}/polls", {
      params: { path: { channel: channelId } },
      body: request,
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.addPoll(result.data);
    return result.data;
  }

  /**
   * Casts the caller's vote. Only their own choice is recorded here; the tally comes by event,
   * which the server publishes before answering.
   */
  async vote(pollId: string, option: number): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/polls/{poll}/votes/{option}/@me", {
      params: { path: { poll: pollId, option } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.setMyVote(pollId, option, true);
  }

  /** Withdraws the caller's vote for an option. */
  async unvote(pollId: string, option: number): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/polls/{poll}/votes/{option}/@me", {
      params: { path: { poll: pollId, option } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.setMyVote(pollId, option, false);
  }

  /** Fetches a poll the cache lacks, with the caller's votes on it, once. */
  ensurePoll(pollId: string): void {
    if (
      this.store.poll(pollId) !== undefined ||
      this.#missingPolls.has(pollId) ||
      this.#pollLoads.has(pollId)
    ) {
      return;
    }
    const load = this.#client.api
      .GET("/api/v1/polls/{poll}", {
        params: { path: { poll: pollId }, query: { include: ["votes"] } },
      })
      .then(({ data, response }) => {
        if (data !== undefined) {
          this.store.ingest({ ...data.included, polls: [data.data] });
        } else if (response.status === 404) {
          this.#missingPolls.add(pollId);
        }
      })
      .catch(() => {
        // Transient; the next render that needs the poll asks again.
      })
      .finally(() => {
        this.#pollLoads.delete(pollId);
      });
    this.#pollLoads.set(pollId, load);
  }

  /** Deletes a message and drops it from the cache at once. */
  async deleteMessage(messageId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/messages/{message}", {
      params: { path: { message: messageId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({ serverEvent: "message", type: "delete", id: messageId });
  }

  /**
   * Changes the caller's own profile: any of the display name, pronouns, bio, and status, with
   * `null` clearing one. The cache is left to the update event, which the server publishes
   * before it answers.
   */
  /** How loud `userId` is to this user on this install: a gain, 1 as sent. Silent while muted for them. */
  async setUserVolume(userId: string, gain: number): Promise<void> {
    await this.preferences.set(userVolume(userId), gain);
    this.voice.setUserVolume(userId, effectiveUserVolume(this.preferences, userId));
  }

  /** Silences `userId` for this user alone, or hears them again at their volume. */
  async setUserMuted(userId: string, muted: boolean): Promise<void> {
    await this.preferences.set(userMuted(userId), muted);
    this.voice.setUserVolume(userId, effectiveUserVolume(this.preferences, userId));
  }

  /** Server-mutes or unmutes someone in a channel's call; their `update` event confirms it. */
  async muteVoiceParticipant(channelId: string, userId: string, muted: boolean): Promise<void> {
    const result = await this.#client.api.PATCH(
      "/api/v1/channels/{channel}/voice/participants/{user}",
      { params: { path: { channel: channelId, user: userId } }, body: { muted } },
    );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Removes someone from a channel's call; their participant `delete` event confirms it. */
  async kickVoiceParticipant(channelId: string, userId: string): Promise<void> {
    const result = await this.#client.api.DELETE(
      "/api/v1/channels/{channel}/voice/participants/{user}",
      { params: { path: { channel: channelId, user: userId } } },
    );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  async updateProfile(patch: UserUpdateRequest): Promise<User> {
    const result = await this.#client.api.PATCH("/api/v1/users/{user}", {
      params: { path: { user: "@me" } },
      body: patch,
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    return result.data;
  }

  /** Creates a category at the end of its community's sort order and caches it at once. */
  async createCategory(communityId: string, name: string): Promise<Category> {
    const sortIndex = this.store
      .categories(communityId)
      .reduce((max, category) => Math.max(max, category.sortIndex + 1), 0);
    const result = await this.#client.api.POST("/api/v1/communities/{community}/categories", {
      params: { path: { community: communityId } },
      body: { name, sortIndex },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ categories: [result.data] });
    return result.data;
  }

  /** Loads a community's invites into the store. */
  async loadInvites(communityId: string): Promise<void> {
    const result = await this.#client.api.GET("/api/v1/communities/{community}/invites", {
      params: { path: { community: communityId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.replaceInvites(communityId, result.data);
  }

  /** Creates an invite. `expiresAt` is an RFC 3339 timestamp, or `null` for a permanent invite. */
  async createInvite(
    communityId: string,
    options: { expiresAt: string | null; customCode?: string },
  ): Promise<Invite> {
    const body: components["schemas"]["InviteCreateRequest"] = { expiresAt: options.expiresAt };
    if (options.customCode !== undefined) {
      body.customCode = options.customCode;
    }
    const result = await this.#client.api.POST("/api/v1/communities/{community}/invites", {
      params: { path: { community: communityId } },
      body,
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.upsertInvite(result.data);
    return result.data;
  }

  async revokeInvite(code: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/invites/{code}", {
      params: { path: { code } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.removeInvite(code);
  }

  /** Resolves an invite code to the community it opens, for the join screen. */
  async lookupInvite(code: string): Promise<InviteLookup> {
    const result = await this.#client.api.GET("/api/v1/invites/{code}", {
      params: { path: { code }, query: { include: ["community"] } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const community = result.data.included.communities?.[0];
    if (community === undefined) {
      throw new ApiProblemError(transportProblem("invite read did not include its community"));
    }
    const invite = result.data.data;
    const expiresAt = invite.expiresAt;
    return {
      invite,
      community,
      expired: expiresAt != null && Date.parse(expiresAt) < this.#now(),
      member: this.store.communities().some((c) => c.id === community.id),
    };
  }

  /**
   * Joins a community with an invite code, then loads it so its channels can be shown at once
   * rather than waiting for the next bootstrap. Joining a community the caller already belongs
   * to succeeds.
   */
  async joinCommunity(communityId: string, inviteCode: string): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/communities/{community}/members/@me", {
      params: { path: { community: communityId } },
      body: { inviteCode },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({ serverEvent: "userCommunity", type: "create", ...result.data });
    await this.loadCommunity(communityId);
  }

  /** Reads one community with its channels, categories, and members into the store. */
  async loadCommunity(communityId: string): Promise<void> {
    const result = await this.#client.api.GET("/api/v1/communities/{community}", {
      params: {
        path: { community: communityId },
        query: { include: ["channels", "categories", "members", "voice"] },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ ...result.data.included, communities: [result.data.data] });
  }

  /**
   * Fetches a user the cache lacks, once. Member samples are capped, so the author of a message
   * that arrives by event may be unknown; this fills the gap on demand.
   */
  ensureUser(userId: string): void {
    if (
      this.store.user(userId) !== undefined ||
      this.#missingUsers.has(userId) ||
      this.#userLoads.has(userId)
    ) {
      return;
    }
    const load = this.#client.api
      .GET("/api/v1/users/{user}", { params: { path: { user: userId } } })
      .then(({ data, response }) => {
        if (data !== undefined) {
          this.store.ingest({ users: [data] });
        } else if (response.status === 404) {
          this.#missingUsers.add(userId);
        }
      })
      .catch(() => {
        // Transient; the next render that needs the user asks again.
      })
      .finally(() => {
        this.#userLoads.delete(userId);
      });
    this.#userLoads.set(userId, load);
  }

  // ---------------------------------------------------------------------------------------

  #setStatus(status: SyncStatus): void {
    if (this.#status === status) {
      return;
    }
    this.#status = status;
    for (const listener of Array.from(this.#listeners)) {
      listener();
    }
    if (status === "live") {
      void this.#pollPresence();
    }
  }

  /** Reads the caller and their communities into the store. Returns whether it succeeded. */
  async #bootstrap(generation: number): Promise<boolean> {
    this.#held = [];
    const startedAt = this.#now();
    try {
      const [me, communities, dms] = await Promise.all([
        this.#client.api.GET("/api/v1/users/{user}", { params: { path: { user: "@me" } } }),
        this.#client.api.GET("/api/v1/users/{user}/communities", {
          params: {
            path: { user: "@me" },
            query: { include: ["channels", "categories", "members", "voice"] },
          },
        }),
        this.#client.api.GET("/api/v1/users/@me/dms", {
          params: { query: { include: ["users"] } },
        }),
      ]);
      if (generation !== this.#generation) {
        return false;
      }
      if (me.data === undefined) {
        throw new ApiProblemError(problemOf(me.error, me.response));
      }
      if (communities.data === undefined) {
        throw new ApiProblemError(problemOf(communities.error, communities.response));
      }
      if (dms.data === undefined) {
        throw new ApiProblemError(problemOf(dms.error, dms.response));
      }
      this.store.setBootstrap(me.data, communities.data.data, communities.data.included);
      this.store.ingest(dms.data.included);
      this.store.setDms(dms.data.data);
      await this.preferences.loadAccount();
      this.#bootstrappedAt = startedAt;
      const held = this.#held;
      this.#held = null;
      for (const event of held) {
        this.#apply(event);
      }
      return true;
    } catch (error) {
      if (generation !== this.#generation) {
        return false;
      }
      this.#held = null;
      this.#lastError =
        error instanceof ApiProblemError ? error.problem : problemOf(error, undefined);
      this.#setStatus("failed");
      return false;
    }
  }

  /**
   * Asks the server for the presence of everyone on screen, then again after
   * `PRESENCE_POLL_MS` for as long as the sync stays live. A hidden page skips the request.
   */
  async #pollPresence(): Promise<void> {
    if (this.#presenceTimer !== null) {
      clearTimeout(this.#presenceTimer);
      this.#presenceTimer = null;
    }
    if (this.#status !== "live") {
      return;
    }
    const hidden = typeof document !== "undefined" && document.visibilityState === "hidden";
    if (!hidden) {
      const ids = this.store.presenceCandidates();
      const batches: string[][] = [];
      for (let i = 0; i < ids.length; i += PRESENCE_BATCH) {
        batches.push(ids.slice(i, i + PRESENCE_BATCH));
      }
      await Promise.all(
        batches.map(async (batch) => {
          const result = await this.#client.api.GET("/api/v1/users/statuses", {
            params: { query: { ids: batch.join(",") } },
          });
          if (result.data !== undefined) {
            this.store.applyStatuses(result.data);
          }
        }),
      ).catch(() => undefined);
    }
    // Read afresh after the awaits, where narrowing is stale.
    if (this.#isLive() && !this.#presencePollScheduled()) {
      this.#presenceTimer = this.#setTimeout(() => {
        this.#presenceTimer = null;
        void this.#pollPresence();
      }, PRESENCE_POLL_MS);
    }
  }

  #isLive(): boolean {
    return this.#status === "live";
  }

  #presencePollScheduled(): boolean {
    return this.#presenceTimer !== null;
  }

  async #resync(): Promise<void> {
    if (this.#status === "stopped" || this.#held !== null) {
      return;
    }
    this.#setStatus("resyncing");
    const generation = this.#generation;
    if (await this.#bootstrap(generation)) {
      this.#setStatus("live");
    }
  }

  /**
   * The user did something in the app. The server hears of it at most once every
   * `ACTIVITY_INTERVAL_MS`, which keeps them showing as online rather than away; activity while
   * the stream is down is reported when it comes back, if it is still recent.
   */
  noteActivity(): void {
    this.#lastActivityAt = this.#now();
    this.#reportActivity();
  }

  #reportActivity(): void {
    const now = this.#now();
    if (
      now - this.#lastActivityAt >= ACTIVITY_INTERVAL_MS ||
      now - this.#activityReportedAt < ACTIVITY_INTERVAL_MS
    ) {
      return;
    }
    if (this.#stream.sendActivity()) {
      this.#activityReportedAt = now;
    }
  }

  #onReady(resumed: boolean): void {
    this.#reportActivity();
    if (this.#status === "resyncing" || this.#status === "failed") {
      return;
    }
    if (!resumed && this.#now() - this.#bootstrappedAt > BOOTSTRAP_STALE_AFTER_MS) {
      // The stream replays a fixed window before its connection; a bootstrap older than that
      // may predate events the replay no longer holds.
      void this.#resync();
      return;
    }
    this.#setStatus("live");
  }

  #onEvent(event: ServerEvent): void {
    if (this.#held !== null) {
      this.#held.push(event);
      return;
    }
    this.#apply(event);
  }

  #apply(event: ServerEvent): void {
    this.store.applyEvent(event);
    if (event.serverEvent === "message" && event.type === "create") {
      this.ensureUser(event.author);
    }
    if (event.serverEvent === "userPreferencesChanged") {
      // Another of the user's devices changed something; the values are fetched rather than
      // carried by the event, so they never reach anyone else's stream.
      if (event.user === this.store.me()?.id) {
        void this.preferences.loadAccount().catch(() => undefined);
      }
      return;
    }
    if (event.serverEvent === "voiceSessionEnded") {
      this.voice.onSessionEnded(event);
    }
  }

  #loadWindow(channelId: string, load: () => Promise<void>): Promise<void> {
    const pending = this.#windowLoads.get(channelId);
    if (pending !== undefined) {
      return pending;
    }
    const generation = this.#generation;
    const promise = load()
      .catch((error: unknown) => {
        if (generation === this.#generation) {
          throw error;
        }
      })
      .finally(() => {
        if (this.#windowLoads.get(channelId) === promise) {
          this.#windowLoads.delete(channelId);
        }
      });
    this.#windowLoads.set(channelId, promise);
    return promise;
  }

  async #readMessages(
    channelId: string,
    query: { before?: string; after?: string; around?: string; limit: number },
  ): Promise<Message[]> {
    const result = await this.#client.api.GET("/api/v1/channels/{channel}/messages", {
      params: {
        path: { channel: channelId },
        query: { ...query, include: ["authors", "attachments", "polls", "threads", "echoes"] },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest(result.data.included);
    return result.data.data;
  }
}

/**
 * The browser media, created only when a call first needs it, so the sync layer can be built
 * where there is no browser at all.
 */
function lazyBrowserMedia(): VoiceMedia {
  let real: VoiceMedia | null = null;
  const media = async (): Promise<VoiceMedia> => {
    if (real === null) {
      const { browserVoiceMedia } = await import("./browserMedia");
      real = browserVoiceMedia();
    }
    return real;
  };
  return {
    createDevice: async () => (await media()).createDevice(),
    getMicrophone: async (choice) => (await media()).getMicrophone(choice),
    setOutput: async (choice) => (await media()).setOutput(choice),
    getScreen: async () => (await media()).getScreen(),
    setVolume: (consumerId, gain) => {
      if (real !== null) {
        real.setVolume(consumerId, gain);
      }
    },
    play: (id, track) => {
      real?.play(id, track);
    },
    stop: (id) => {
      real?.stop(id);
    },
  };
}

/** The page's `localStorage`, when there is one and it can be touched. */
function pageStorage(): PreferenceStorage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}
