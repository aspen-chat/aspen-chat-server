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
import type { components, paths } from "./generated/openapi";
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
import { REACTION_SUMMARY_USERS, RecordStore, type NotificationLevel } from "./store";
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
export type AdminOverview = components["schemas"]["AdminOverview"];
export type AdminUserEntry = components["schemas"]["AdminUserEntry"];
export type AdminCommunityEntry = components["schemas"]["AdminCommunityEntry"];
export type RegistrationInvite = components["schemas"]["RegistrationInvite"];
export type RegistrationInviteRequest = components["schemas"]["RegistrationInviteRequest"];
export type Fleet = components["schemas"]["Fleet"];
export type ApiServerHealth = components["schemas"]["ApiServerHealth"];
export type VoiceServerHealth = components["schemas"]["VoiceServerHealth"];
export type FederationOverview = components["schemas"]["FederationOverview"];
export type FederatedDeployment = components["schemas"]["FederatedDeployment"];
export type FederationList = components["schemas"]["FederationList"];
export type ContactResult = components["schemas"]["ContactResult"];
export type Gate = components["schemas"]["Gate"];

export type UserSort = NonNullable<
  NonNullable<paths["/api/v1/admin/users"]["get"]["parameters"]["query"]>["sort"]
>;
export type CommunitySort = NonNullable<
  NonNullable<paths["/api/v1/admin/communities"]["get"]["parameters"]["query"]>["sort"]
>;
export type Growth = components["schemas"]["Growth"];
export type DeploymentRole = components["schemas"]["DeploymentRole"];
export type DeploymentPermission = components["schemas"]["DeploymentPermission"];
export type ModerationEntry = components["schemas"]["ModerationEntry"];
export type FileOfferEntry = components["schemas"]["FileOfferEntry"];
export type GrowthRange = paths["/api/v1/admin/growth"]["get"]["parameters"]["query"]["range"];
export type MessageHolding = components["schemas"]["MessageHolding"];

/** What to search messages for (`AspenSync.searchMessages`); at least one of the first four. */
export interface MessageSearch {
  /** Words, in web search syntax: `"a phrase"`, `or`, `-left out`. */
  text?: string;
  author?: string;
  /** Messages tagging this user by name. */
  mentions?: string;
  has?: readonly MessageHolding[];
  /** Only this community; or `channel`, only that channel or DM and its threads. */
  community?: string;
  channel?: string;
  /** The last message of the previous page. */
  before?: string;
}

/** How many messages one page of search results holds. */
export const SEARCH_PAGE = 25;

/** A page of one of the dashboard's lists. */
export interface AdminListQuery<S extends string> {
  /** Only those whose names contain this, ignoring case. */
  name?: string;
  /** The order; newest first when absent. */
  sort?: S;
  /** How many rows to skip. */
  offset?: number;
  /** How many rows the page holds. */
  limit?: number;
}

function listQuery<S extends string>(
  query: AdminListQuery<S>,
): { "filter[name]"?: string; sort?: S; offset?: number; limit?: number } {
  return {
    ...(query.name === undefined || query.name.trim() === ""
      ? {}
      : { "filter[name]": query.name.trim() }),
    ...(query.sort === undefined ? {} : { sort: query.sort }),
    ...(query.offset === undefined || query.offset === 0 ? {} : { offset: query.offset }),
    ...(query.limit === undefined ? {} : { limit: query.limit }),
  };
}
type Icon = components["schemas"]["Icon"];
type CommunityUpdateRequest = components["schemas"]["CommunityUpdateRequest"];
type UserUpdateRequest = components["schemas"]["UserUpdateRequest"];
type Role = components["schemas"]["Role"];
type Permission = components["schemas"]["Permission"];

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
 * How long reading a channel is gathered before it is reported: one report per channel per
 * this long however fast messages scroll past, well inside the server's limit on reports.
 */
export const READ_REPORT_MS = 1_000;

/** How many people a page of a reaction list holds. */
/**
 * How far before this sync began a message may say it was posted and still notify, allowing for
 * the server's clock and the device's disagreeing; well inside the minute a connection replays.
 */
const NOTIFY_CLOCK_SLACK_MS = 5_000;

export const REACTORS_PAGE = 50;

/** The longest delay `setTimeout` keeps; a longer one fires at once. */
const MAX_TIMER_MS = 2 ** 31 - 1;
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
  /**
   * The preferences to use instead of the server's own: the home deployment's, for a sync of
   * another deployment, since preferences are the user's and kept at home. This sync then
   * neither loads nor clears the account's preferences.
   */
  preferences?: PreferenceStore;
}

export type SyncListener = () => void;

/**
 * Another deployment the user signs in to says they are now in a DM there (the home's
 * `foreignDmJoined` event). `channel` is that deployment's id.
 */
export interface ForeignDmNotice {
  readonly domain: string;
  readonly channel: string;
  readonly byName: string;
  readonly byDisplayName: string | null;
}

/** How many members one page of a member search holds. */
export const MEMBER_SEARCH_PAGE = 20;

/** How long a read after a change to the caller's access may wait, at most. */
export const ACCESS_RELOAD_SPREAD_MS = 2000;

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
  /** Per channel, the furthest message read and not yet reported. */
  readonly #unreported = new Map<string, string>();
  #readTimer: ReturnType<typeof setTimeout> | null = null;
  #muteTimer: ReturnType<typeof setTimeout> | null = null;
  readonly #missingPolls = new Set<string>();
  readonly #iconLoads = new Map<string, Promise<void>>();
  readonly #missingIcons = new Set<string>();
  readonly #random: () => number;
  /** Communities waiting to be read again because the caller's access in them may have grown. */
  readonly #accessReloads = new Set<string>();
  readonly #pinLoads = new Map<string, Promise<void>>();
  readonly #foreignDmListeners = new Set<(notice: ForeignDmNotice) => void>();
  readonly #notifyListeners = new Set<(message: Message) => void>();
  /** Whether `preferences` is this sync's own, loaded from and cleared with its server. */
  readonly #ownsPreferences: boolean;

  constructor(options: AspenSyncOptions) {
    this.#client = options.client;
    this.store = options.store ?? new RecordStore({ now: options.now ?? (() => Date.now()) });
    this.#now = options.now ?? (() => Date.now());
    this.#uploadFetch = options.uploadFetch ?? ((input, init) => globalThis.fetch(input, init));
    this.#random = options.random ?? Math.random;
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
    voiceOptions.userVolume = (userId) => this.#userGain(userId);
    this.voice = new VoiceCall(voiceOptions);
    // A block made or lifted, here or on another deployment, changes who is heard at once.
    this.store.subscribe("silenced", () => {
      this.voice.refreshVolumes();
    });
    this.#setTimeout = options.setTimeout ?? globalThis.setTimeout.bind(globalThis);
    if (typeof document !== "undefined") {
      // A page coming back into view gets fresh presence at once rather than at the next tick.
      document.addEventListener("visibilitychange", () => {
        if (document.visibilityState === "visible" && this.#status === "live") {
          void this.#pollPresence();
        }
      });
    }
    this.#ownsPreferences = options.preferences === undefined;
    this.preferences =
      options.preferences ??
      new PreferenceStore({
        storage:
          options.preferenceStorage === undefined ? pageStorage() : options.preferenceStorage,
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

  /**
   * Registers for the home's word that the user is now in a DM on another deployment, and
   * returns the unsubscribe function. What to do about it is the app's: sign in there if it is
   * not, and read the DM.
   */
  readonly onForeignDm = (listener: (notice: ForeignDmNotice) => void): (() => void) => {
    this.#foreignDmListeners.add(listener);
    return () => {
      this.#foreignDmListeners.delete(listener);
    };
  };

  /**
   * Registers for new messages the user's notification settings say to tell them of
   * (`RecordStore.notifies`), as they arrive, and returns the unsubscribe function. How to tell
   * them is the app's.
   */
  readonly onNotify = (listener: (message: Message) => void): (() => void) => {
    this.#notifyListeners.add(listener);
    return () => {
      this.#notifyListeners.delete(listener);
    };
  };

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
    if (this.#ownsPreferences) {
      this.preferences.clearAccount();
    }
    this.#held = null;
    this.#windowLoads.clear();
    this.#userLoads.clear();
    this.#pollLoads.clear();
    this.#iconLoads.clear();
    this.#unreported.clear();
    if (this.#readTimer !== null) {
      clearTimeout(this.#readTimer);
      this.#readTimer = null;
    }
    if (this.#muteTimer !== null) {
      clearTimeout(this.#muteTimer);
      this.#muteTimer = null;
    }
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
    if (!this.#readsEventsOf(communityId)) {
      this.store.applyEvent({
        serverEvent: "community",
        type: "update",
        id: communityId,
        ...patch,
      });
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
        query: { include: ["authors", "attachments", "polls", "threads", "reactions"] },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ ...result.data.included, messages: [result.data.data] });
    this.store.setReactions([result.data.data.id], result.data.included.reactions ?? []);
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
        // The membership's own event, which follows, names the roles the creator was given.
        roles: [],
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

  /**
   * Everyone who reacted to a message with an emoji, earliest first, a page at a time: the page
   * after `after`, or the first. A page shorter than `REACTORS_PAGE` is the last. The users are
   * stored as they come.
   */
  async loadReactors(messageId: string, emoji: string, after?: string): Promise<User[]> {
    const result = await this.#client.api.GET("/api/v1/messages/{message}/reactions/{emoji}", {
      params: {
        path: { message: messageId, emoji },
        query: after === undefined ? { limit: REACTORS_PAGE } : { after, limit: REACTORS_PAGE },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ users: result.data });
    return result.data;
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

  /**
   * Adds the caller's own answer to a poll that allows write-ins, which also votes for it for
   * them. An answer the poll already has is voted for instead of added. Resolves to the option
   * the vote went to. The poll's new answers and tally come by event.
   */
  async writeIn(pollId: string, label: string): Promise<number> {
    const result = await this.#client.api.POST("/api/v1/polls/{poll}/write-ins", {
      params: { path: { poll: pollId } },
      body: { label },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const { option } = result.data;
    this.store.setMyVote(pollId, option, true);
    if (result.response.status === 201) {
      this.store.setMyWriteIn(pollId, option, true);
    }
    return option;
  }

  /** Removes a written-in answer, and every vote for it. */
  /**
   * Records that the caller has seen `messageId` in `channelId`: at once in the store, and to
   * the server within `READ_REPORT_MS`, together with whatever else was read meanwhile. Reading
   * behind the current position changes nothing.
   */
  markRead(channelId: string, messageId: string): void {
    const state = this.store.readState(channelId);
    if (state === undefined || messageId <= state.lastRead) {
      return;
    }
    this.store.setLastRead(channelId, messageId);
    this.#unreported.set(channelId, messageId);
    this.#readTimer ??= setTimeout(() => {
      this.flushReads();
    }, READ_REPORT_MS);
  }

  /**
   * Mutes a channel or DM for the caller, for `durationSeconds` or, with `null`, until they
   * unmute it. The store follows the `channelMuteChanged` event.
   */
  async muteChannel(channelId: string, durationSeconds: number | null): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/channels/{channel}/mutes/@me", {
      params: { path: { channel: channelId } },
      body: { durationSeconds },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Sets how much of a channel (a text channel or DM) the user is told of, or with `null`
   * returns it to its community's setting or the default. The store follows at once; the
   * `notificationSettingChanged` event that follows changes nothing more.
   */
  async setChannelNotifications(channelId: string, level: NotificationLevel | null): Promise<void> {
    const path = { params: { path: { channel: channelId } } };
    const result =
      level === null
        ? await this.#client.api.DELETE(
            "/api/v1/channels/{channel}/notification-settings/@me",
            path,
          )
        : await this.#client.api.PUT("/api/v1/channels/{channel}/notification-settings/@me", {
            ...path,
            body: { level },
          });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({
      serverEvent: "notificationSettingChanged",
      community: null,
      channel: channelId,
      level,
    });
  }

  /** Sets how much of a community's channels without a setting of their own the user is told of. */
  async setCommunityNotifications(
    communityId: string,
    level: NotificationLevel | null,
  ): Promise<void> {
    const path = { params: { path: { community: communityId } } };
    const result =
      level === null
        ? await this.#client.api.DELETE(
            "/api/v1/communities/{community}/notification-settings/@me",
            path,
          )
        : await this.#client.api.PUT("/api/v1/communities/{community}/notification-settings/@me", {
            ...path,
            body: { level },
          });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({
      serverEvent: "notificationSettingChanged",
      community: communityId,
      channel: null,
      level,
    });
  }

  // --- The Administration Dashboard. Its reads are queries rather than cached records: each
  // answers what the server says now, and the dashboard holds the answer while it shows it.

  /** The deployment's totals. */
  async adminOverview(): Promise<AdminOverview> {
    return this.#adminRead(await this.#client.api.GET("/api/v1/admin/overview"));
  }

  /** A page of the deployment's users, searched and sorted. */
  async adminUsers(query: AdminListQuery<UserSort> = {}): Promise<AdminUserEntry[]> {
    return this.#adminRead(
      await this.#client.api.GET("/api/v1/admin/users", { params: { query: listQuery(query) } }),
    );
  }

  /** A page of the deployment's communities, searched and sorted. */
  async adminCommunities(
    query: AdminListQuery<CommunitySort> = {},
  ): Promise<AdminCommunityEntry[]> {
    return this.#adminRead(
      await this.#client.api.GET("/api/v1/admin/communities", {
        params: { query: listQuery(query) },
      }),
    );
  }

  /** The newest registration invites, usable or not. */
  async registrationInvites(): Promise<RegistrationInvite[]> {
    return this.#adminRead(await this.#client.api.GET("/api/v1/admin/registration-invites"));
  }

  /** Makes a registration invite. */
  async createRegistrationInvite(request: RegistrationInviteRequest): Promise<RegistrationInvite> {
    return this.#adminRead(
      await this.#client.api.POST("/api/v1/admin/registration-invites", { body: request }),
    );
  }

  /** Revokes a registration invite; the accounts it made are kept. */
  async revokeRegistrationInvite(code: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/admin/registration-invites/{code}", {
      params: { path: { code } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** This deployment's part in federation: its domain, key, gates, and lists in force. */
  async federation(): Promise<FederationOverview> {
    return this.#adminRead(await this.#client.api.GET("/api/v1/admin/federation"));
  }

  /** A page of the other deployments this one knows, alphabetically, searched by domain. */
  async federatedDeployments(
    query: Omit<AdminListQuery<never>, "sort"> = {},
  ): Promise<FederatedDeployment[]> {
    return this.#adminRead(
      await this.#client.api.GET("/api/v1/admin/federation/deployments", {
        params: { query: listQuery(query) },
      }),
    );
  }

  /** Adds a deployment to the directory, not yet contacted. */
  async addFederatedDeployment(domain: string, note?: string): Promise<FederatedDeployment> {
    return this.#adminRead(
      await this.#client.api.POST("/api/v1/admin/federation/deployments", {
        body: {
          domain,
          ...(note === undefined || note.trim() === "" ? {} : { note: note.trim() }),
        },
      }),
    );
  }

  /** Changes or, with `null`, clears the note kept on a deployment. */
  async setFederatedDeploymentNote(
    domain: string,
    note: string | null,
  ): Promise<FederatedDeployment> {
    return this.#adminRead(
      await this.#client.api.PATCH("/api/v1/admin/federation/deployments/{domain}", {
        params: { path: { domain } },
        body: { note },
      }),
    );
  }

  /** Forgets a deployment: its pinned key and the lists it is on. */
  async removeFederatedDeployment(domain: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/admin/federation/deployments/{domain}", {
      params: { path: { domain } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Reads a deployment's document now, pinning or checking its key. */
  async contactFederatedDeployment(domain: string): Promise<ContactResult> {
    return this.#adminRead(
      await this.#client.api.POST("/api/v1/admin/federation/deployments/{domain}/contact", {
        params: { path: { domain } },
      }),
    );
  }

  /** Accepts the key a deployment offers in place of its pinned one: exactly `publicKey`. */
  async acceptFederatedDeploymentKey(
    domain: string,
    publicKey: string,
  ): Promise<FederatedDeployment> {
    return this.#adminRead(
      await this.#client.api.PUT("/api/v1/admin/federation/deployments/{domain}/key", {
        params: { path: { domain } },
        body: { publicKey },
      }),
    );
  }

  /** Puts a deployment on a list or takes it off. */
  async setFederationListed(domain: string, list: FederationList, listed: boolean): Promise<void> {
    const params = { params: { path: { domain, list } } };
    const result = listed
      ? await this.#client.api.PUT(
          "/api/v1/admin/federation/deployments/{domain}/lists/{list}",
          params,
        )
      : await this.#client.api.DELETE(
          "/api/v1/admin/federation/deployments/{domain}/lists/{list}",
          params,
        );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** How many users and communities there were at each step of `range`. */
  async adminGrowth(range: GrowthRange): Promise<Growth> {
    return this.#adminRead(
      await this.#client.api.GET("/api/v1/admin/growth", { params: { query: { range } } }),
    );
  }

  /** The health of the deployment's API and voice servers. */
  async fleet(): Promise<Fleet> {
    return this.#adminRead(await this.#client.api.GET("/api/v1/admin/fleet"));
  }

  /** What the caller may do across the deployment, and the roles that give it. */
  async deploymentAccess(): Promise<{ permissions: DeploymentPermission[]; roles: string[] }> {
    return this.#adminRead(await this.#client.api.GET("/api/v1/users/@me/admin"));
  }

  /** The deployment's roles, lowest first, as a query of the moment. */
  async deploymentRoles(): Promise<DeploymentRole[]> {
    return this.#adminRead(await this.#client.api.GET("/api/v1/admin/roles"));
  }

  async createDeploymentRole(
    name: string,
    permissions: readonly DeploymentPermission[],
  ): Promise<DeploymentRole> {
    return this.#adminRead(
      await this.#client.api.POST("/api/v1/admin/roles", {
        body: { name, permissions: [...permissions] },
      }),
    );
  }

  async updateDeploymentRole(
    roleId: string,
    patch: { name?: string; permissions?: readonly DeploymentPermission[] },
  ): Promise<DeploymentRole> {
    return this.#adminRead(
      await this.#client.api.PATCH("/api/v1/admin/roles/{role}", {
        params: { path: { role: roleId } },
        body: {
          ...(patch.name !== undefined ? { name: patch.name } : {}),
          ...(patch.permissions !== undefined ? { permissions: [...patch.permissions] } : {}),
        },
      }),
    );
  }

  async deleteDeploymentRole(roleId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/admin/roles/{role}", {
      params: { path: { role: roleId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Orders the deployment roles below the caller's highest, lowest first. */
  async reorderDeploymentRoles(roleIds: readonly string[]): Promise<DeploymentRole[]> {
    return this.#adminRead(
      await this.#client.api.PUT("/api/v1/admin/role-order", { body: { roles: [...roleIds] } }),
    );
  }

  /** Gives someone a deployment role, or takes it away. */
  async setUserDeploymentRole(userId: string, roleId: string, held: boolean): Promise<void> {
    const params = { params: { path: { user: userId, role: roleId } } };
    const result = held
      ? await this.#client.api.PUT("/api/v1/admin/users/{user}/roles/{role}", params)
      : await this.#client.api.DELETE("/api/v1/admin/users/{user}/roles/{role}", params);
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** A page of the moderation log, newest first. */
  async moderationLog(before?: string): Promise<ModerationEntry[]> {
    return this.#adminRead(
      await this.#client.api.GET("/api/v1/admin/moderation-log", {
        params: { query: before === undefined ? {} : { before } },
      }),
    );
  }

  /** A page of the record of files offered in calls, newest first, optionally one user's. */
  async fileTransferLog(before?: string, user?: string): Promise<FileOfferEntry[]> {
    return this.#adminRead(
      await this.#client.api.GET("/api/v1/admin/file-transfers", {
        params: {
          query: {
            ...(before === undefined ? {} : { before }),
            ...(user === undefined ? {} : { "filter[user]": user }),
          },
        },
      }),
    );
  }

  /**
   * Someone's DMs, for a deployment moderator to open; they are put in the store so the DM
   * screen can show one, and reading any is logged by the server.
   */
  async userDms(userId: string): Promise<Channel[]> {
    const dms = this.#adminRead(
      await this.#client.api.GET("/api/v1/admin/users/{user}/dms", {
        params: { path: { user: userId } },
      }),
    );
    this.store.ingest({ channels: dms });
    for (const dm of dms) {
      for (const recipient of dm.recipients) {
        this.ensureUser(recipient);
      }
    }
    return dms;
  }

  /** Takes someone else's reaction off a message; see `#readsEventsOf` for the cache. */
  async removeUsersReaction(messageId: string, emoji: string, userId: string): Promise<void> {
    const result = await this.#client.api.DELETE(
      "/api/v1/messages/{message}/reactions/{emoji}/{user}",
      { params: { path: { message: messageId, emoji, user: userId } } },
    );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const channel = this.store.message(messageId)?.channelId;
    if (!this.#readsEventsOf(this.store.channel(channel ?? "")?.community ?? null)) {
      this.store.applyEvent({ serverEvent: "react", type: "delete", messageId, emoji, userId });
    }
  }

  /** Takes an attachment off a message, and reads the message again for what is left. */
  async removeAttachment(messageId: string, attachmentId: string): Promise<void> {
    const result = await this.#client.api.DELETE(
      "/api/v1/messages/{message}/attachments/{attachment}",
      { params: { path: { message: messageId, attachment: attachmentId } } },
    );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    await this.loadMessage(messageId);
  }

  /** Renames a channel; see `#readsEventsOf` for the cache. */
  async renameChannel(channelId: string, name: string): Promise<void> {
    const result = await this.#client.api.PATCH("/api/v1/channels/{channel}", {
      params: { path: { channel: channelId } },
      body: { name },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    if (!this.#readsEventsOf(this.store.channel(channelId)?.community ?? null)) {
      this.store.applyEvent({ serverEvent: "channel", type: "update", id: channelId, name });
    }
  }

  /** Deletes a channel; see `#readsEventsOf` for the cache. */
  async deleteChannel(channelId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/channels/{channel}", {
      params: { path: { channel: channelId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    if (!this.#readsEventsOf(this.store.channel(channelId)?.community ?? null)) {
      this.store.applyEvent({ serverEvent: "channel", type: "delete", id: channelId });
    }
  }

  /**
   * Whether the event stream brings what happens in a community: it does for one the caller
   * belongs to, where a write's own event updates the cache. A moderator acting in one they are
   * not in, or in a DM (`null`) they are not in, reads no events of it, so such a write applies
   * its change to the cache itself.
   */
  #readsEventsOf(communityId: string | null): boolean {
    if (communityId === null) {
      return false;
    }
    return this.store.communities().some((c) => c.id === communityId);
  }

  #adminRead<T>(result: { data?: T; error?: unknown; response: Response }): T {
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    return result.data;
  }

  /** Collapses or expands a category in the caller's channel list. The store follows the event. */
  async setCategoryCollapsed(categoryId: string, collapsed: boolean): Promise<void> {
    const params = { params: { path: { category: categoryId } } };
    const result = collapsed
      ? await this.#client.api.PUT("/api/v1/categories/{category}/collapses/@me", params)
      : await this.#client.api.DELETE("/api/v1/categories/{category}/collapses/@me", params);
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Lifts the caller's mute of a channel or DM. The store follows the event. */
  async unmuteChannel(channelId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/channels/{channel}/mutes/@me", {
      params: { path: { channel: channelId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Reports what has been read and not yet reported, now; for a page about to be hidden. */
  flushReads(): void {
    if (this.#readTimer !== null) {
      clearTimeout(this.#readTimer);
      this.#readTimer = null;
    }
    const reports = Array.from(this.#unreported);
    this.#unreported.clear();
    for (const [channelId, messageId] of reports) {
      // A failed report is not retried: the next message read reports a later position.
      void this.#client.api
        .PUT("/api/v1/channels/{channel}/read-states/@me", {
          params: { path: { channel: channelId } },
          body: { lastRead: messageId },
        })
        .then(() => {
          // Read partway, which tags remain is the server's to count.
          this.#recountTags(channelId);
        })
        .catch(() => undefined);
    }
  }

  /** Reads a channel's state again while tags remain in it, since only the server knows which. */
  #recountTags(channelId: string): void {
    if (this.store.mentions(channelId) > 0) {
      void this.#reloadReadState(channelId);
    }
  }

  async #reloadReadState(channelId: string): Promise<void> {
    const generation = this.#generation;
    const result = await this.#client.api
      .GET("/api/v1/channels/{channel}/read-states/@me", {
        params: { path: { channel: channelId } },
      })
      .catch(() => undefined);
    if (result?.data !== undefined && generation === this.#generation) {
      this.store.putReadState(result.data);
    }
  }

  async removeWriteIn(pollId: string, option: number): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/polls/{poll}/write-ins/{option}", {
      params: { path: { poll: pollId, option } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.setMyWriteIn(pollId, option, false);
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
   * How loud `userId` is to this user on this install: a gain, 1 as sent. Silent while muted
   * for them or blocked.
   */
  async setUserVolume(userId: string, gain: number): Promise<void> {
    await this.preferences.set(userVolume(userId), gain);
    this.voice.setUserVolume(userId, this.#userGain(userId));
  }

  /** Silences `userId` for this user alone, or hears them again at their volume. */
  async setUserMuted(userId: string, muted: boolean): Promise<void> {
    await this.preferences.set(userMuted(userId), muted);
    this.voice.setUserVolume(userId, this.#userGain(userId));
  }

  /** How loud `userId` plays here: silent while muted for this user, or blocked on any deployment. */
  #userGain(userId: string): number {
    return this.store.silenced(userId) ? 0 : effectiveUserVolume(this.preferences, userId);
  }

  /**
   * Who the user blocked on every deployment they use, by `identityOf`, with `domain`, this
   * deployment's name: those of them in a call here are silenced and their screens hidden, as
   * if blocked here.
   */
  setBlockedIdentities(domain: string, identities: ReadonlySet<string>): void {
    this.store.setBlockedIdentities(domain, identities);
  }

  /**
   * Blocks someone for the caller: their messages collapse, their reactions go, and they are
   * silenced and hidden in calls. They are not told.
   */
  async blockUser(userId: string): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/users/@me/blocks/{user}", {
      params: { path: { user: userId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    if (this.store.setBlocked(userId, true)) {
      this.#blockChanged();
    }
  }

  /** Lifts the caller's block of someone. */
  async unblockUser(userId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/users/@me/blocks/{user}", {
      params: { path: { user: userId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    if (this.store.setBlocked(userId, false)) {
      this.#blockChanged();
    }
  }

  /** Reads every bot the caller owns into the store (`RecordStore.ownedBots`). */
  async loadBots(): Promise<void> {
    const result = await this.#client.api.GET("/api/v1/users/@me/bots");
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ users: result.data });
  }

  /**
   * Makes a bot the caller owns. Resolves to the bot and its token, which the server shows only
   * this once.
   */
  async createBot(name: string, displayName: string | null): Promise<{ bot: User; token: string }> {
    const result = await this.#client.api.POST("/api/v1/users/@me/bots", {
      body: { name, displayName },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ users: [result.data.bot] });
    return result.data;
  }

  /** Issues a new token for a bot the caller owns; the old one stops working. */
  async rotateBotToken(botId: string): Promise<string> {
    const result = await this.#client.api.POST("/api/v1/bots/{bot}/token", {
      params: { path: { bot: botId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    return result.data.token;
  }

  /** Makes a bot the caller owns public, so anyone allowed may add it, or private. */
  async setBotPublic(botId: string, isPublic: boolean): Promise<void> {
    const result = await this.#client.api.PATCH("/api/v1/bots/{bot}", {
      params: { path: { bot: botId } },
      body: { public: isPublic },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    // The bot's own update event reaches only those who share a community with it.
    this.store.ingest({ users: [result.data] });
  }

  /** Hands a bot the caller owns to someone else; it leaves the caller's list. */
  async transferBot(botId: string, ownerId: string): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/bots/{bot}/owner", {
      params: { path: { bot: botId } },
      body: { owner: ownerId },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ users: [result.data] });
  }

  /** Deletes a bot: the caller's own, or, with Manage bots, one whose owner is gone. */
  /**
   * Bans a user of another deployment from this one, ending their sessions here, or lifts the
   * ban; takes Moderate any community.
   */
  async setForeignUserBanned(userId: string, banned: boolean): Promise<void> {
    const params = { params: { path: { user: userId } } };
    const result = banned
      ? await this.#client.api.PUT("/api/v1/admin/users/{user}/ban", params)
      : await this.#client.api.DELETE("/api/v1/admin/users/{user}/ban", params);
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  async deleteBot(botId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/bots/{bot}", {
      params: { path: { bot: botId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.forgetUser(botId);
  }

  /**
   * Adds a bot to a community, as its link offers, giving it `permissions` on a role of its
   * own. The membership and the role arrive as the community's events.
   */
  async addBot(
    communityId: string,
    botId: string,
    permissions: readonly Permission[],
  ): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/communities/{community}/members/{user}", {
      params: { path: { community: communityId, user: botId } },
      body: { permissions: [...permissions] },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Follows a block made or lifted, here or on another device: what the server counts for the
   * caller alone (unread, reaction summaries) is read again. The call follows the store's
   * `silenced` topic.
   */
  #blockChanged(): void {
    void this.#refreshBlockedCounts();
  }

  /**
   * Reads again what the server leaves blocked users out of: every read state, and the
   * reactions of each held message window, one read per window (a window never holds more than
   * a read returns on each side of its middle).
   */
  async #refreshBlockedCounts(): Promise<void> {
    const generation = this.#generation;
    const [communities, dms] = await Promise.all([
      this.#client.api
        .GET("/api/v1/users/{user}/communities", {
          params: { path: { user: "@me" }, query: { include: ["readStates"] } },
        })
        .catch(() => undefined),
      this.#client.api
        .GET("/api/v1/users/@me/dms", { params: { query: { include: ["readStates"] } } })
        .catch(() => undefined),
    ]);
    if (generation !== this.#generation) {
      return;
    }
    for (const state of [
      ...(communities?.data?.included.readStates ?? []),
      ...(dms?.data?.included.readStates ?? []),
    ]) {
      this.store.putReadState(state);
    }
    await Promise.all(
      this.store.heldWindows().map(async (channelId) => {
        const ids = this.store.messages(channelId)?.ids ?? [];
        const middle = ids[Math.floor(ids.length / 2)];
        if (middle === undefined) {
          return;
        }
        const result = await this.#client.api
          .GET("/api/v1/channels/{channel}/messages", {
            params: {
              path: { channel: channelId },
              query: {
                around: middle,
                limit: Math.ceil(ids.length / 2),
                include: ["reactions"],
              },
            },
          })
          .catch(() => undefined);
        if (result?.data !== undefined && generation === this.#generation) {
          this.store.setReactions(ids, result.data.included.reactions ?? []);
        }
      }),
    );
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

  /**
   * Makes a role, just above everyone's. It is cached from the response at once, so it can be
   * edited before its event arrives; the event then changes nothing.
   */
  async createRole(
    communityId: string,
    name: string,
    permissions: readonly Permission[],
  ): Promise<Role> {
    const result = await this.#client.api.POST("/api/v1/communities/{community}/roles", {
      params: { path: { community: communityId } },
      body: { name, permissions: [...permissions] },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    if (this.store.roles(communityId).every((r) => r.id !== result.data.id)) {
      this.store.applyEvent({ serverEvent: "role", type: "create", ...result.data });
    }
    return result.data;
  }

  /** Renames a role or sets its permissions; its update event changes the cache. */
  async updateRole(
    roleId: string,
    patch: { name?: string; permissions?: readonly Permission[] },
  ): Promise<void> {
    const result = await this.#client.api.PATCH("/api/v1/roles/{role}", {
      params: { path: { role: roleId } },
      body: {
        ...(patch.name !== undefined ? { name: patch.name } : {}),
        ...(patch.permissions !== undefined ? { permissions: [...patch.permissions] } : {}),
      },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  async deleteRole(roleId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/roles/{role}", {
      params: { path: { role: roleId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Orders the roles below the caller's highest, lowest first, everyone's left out. */
  async reorderRoles(communityId: string, roleIds: readonly string[]): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/communities/{community}/role-order", {
      params: { path: { community: communityId } },
      body: { roles: [...roleIds] },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Gives a member a role, or takes it away; their membership's event changes the cache. */
  async setMemberRole(
    communityId: string,
    userId: string,
    roleId: string,
    held: boolean,
  ): Promise<void> {
    const params = { path: { community: communityId, user: userId, role: roleId } };
    const result = held
      ? await this.#client.api.PUT("/api/v1/communities/{community}/members/{user}/roles/{role}", {
          params,
        })
      : await this.#client.api.DELETE(
          "/api/v1/communities/{community}/members/{user}/roles/{role}",
          { params },
        );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * A page of a community's members whose name contains `name`, sorted by name. Only those who
   * act on members may search a community larger than its member sample; the server refuses
   * anyone else. The members are cached, their roles too, but not added to the sample.
   */
  async searchMembers(communityId: string, name: string, offset = 0): Promise<User[]> {
    const result = await this.#client.api.GET("/api/v1/communities/{community}/members", {
      params: {
        path: { community: communityId },
        query: { "filter[name]": name, offset, limit: MEMBER_SEARCH_PAGE },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ users: result.data.data });
    this.store.noteMemberRoles(result.data.included.userCommunities ?? []);
    return result.data.data;
  }

  /** Leaves a community, which drops it from the caller's list at once. */
  async leaveCommunity(communityId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/communities/{community}/members/@me", {
      params: { path: { community: communityId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const me = this.store.myUserId;
    if (me !== null) {
      this.store.applyEvent({
        serverEvent: "userCommunity",
        type: "delete",
        community: communityId,
        user: me,
      });
    }
  }

  /** Removes someone from a community; they may come back with an invite. */
  async removeMember(communityId: string, userId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/communities/{community}/members/{user}", {
      params: { path: { community: communityId, user: userId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Hands the community to another member; only its owner may. */
  async transferOwnership(communityId: string, userId: string): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/communities/{community}/owner", {
      params: { path: { community: communityId } },
      body: { user: userId },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Deletes a community, which its owner or a deployment moderator may; see `#readsEventsOf`. */
  async deleteCommunity(communityId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/communities/{community}", {
      params: { path: { community: communityId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    if (!this.#readsEventsOf(communityId)) {
      this.store.applyEvent({ serverEvent: "community", type: "delete", id: communityId });
    }
  }

  /**
   * Sets a role's override of a channel or category (`kind`), or with `null` clears it. What a
   * permission is neither allowed nor denied in is inherited.
   */
  async setOverride(
    kind: "channel" | "category",
    targetId: string,
    roleId: string,
    set: { allow: readonly Permission[]; deny: readonly Permission[] } | null,
  ): Promise<void> {
    const body = set === null ? null : { allow: [...set.allow], deny: [...set.deny] };
    let result;
    if (kind === "channel") {
      const params = { path: { channel: targetId, role: roleId } };
      result =
        body === null
          ? await this.#client.api.DELETE("/api/v1/channels/{channel}/overrides/{role}", { params })
          : await this.#client.api.PUT("/api/v1/channels/{channel}/overrides/{role}", {
              params,
              body,
            });
    } else {
      const params = { path: { category: targetId, role: roleId } };
      result =
        body === null
          ? await this.#client.api.DELETE("/api/v1/categories/{category}/overrides/{role}", {
              params,
            })
          : await this.#client.api.PUT("/api/v1/categories/{category}/overrides/{role}", {
              params,
              body,
            });
    }
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Pins a message in its channel, or unpins it. */
  async setPinned(messageId: string, pinned: boolean): Promise<void> {
    const params = { path: { message: messageId } };
    const result = pinned
      ? await this.#client.api.PUT("/api/v1/messages/{message}/pin", { params })
      : await this.#client.api.DELETE("/api/v1/messages/{message}/pin", { params });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Reads a channel's pins into the store, once however many ask at the same time; pin events
   * keep them current after.
   */
  loadPins(channelId: string): Promise<void> {
    const pending = this.#pinLoads.get(channelId);
    if (pending !== undefined) {
      return pending;
    }
    const load = (async () => {
      const result = await this.#client.api.GET("/api/v1/channels/{channel}/pins", {
        params: { path: { channel: channelId } },
      });
      if (result.data === undefined) {
        throw new ApiProblemError(problemOf(result.error, result.response));
      }
      this.store.setPins(channelId, result.data);
    })().finally(() => {
      this.#pinLoads.delete(channelId);
    });
    this.#pinLoads.set(channelId, load);
    return load;
  }

  /**
   * The messages matching `search` that the user may read on this deployment, newest first, a
   * page of `SEARCH_PAGE`. Their authors, channels (threads among them), attachments, polls,
   * and reactions are cached, so they render as they do in a channel.
   */
  async searchMessages(search: MessageSearch): Promise<Message[]> {
    const result = await this.#client.api.GET("/api/v1/messages", {
      params: {
        query: {
          ...(search.text === undefined ? {} : { "filter[text]": search.text }),
          ...(search.author === undefined ? {} : { "filter[author]": search.author }),
          ...(search.mentions === undefined ? {} : { "filter[mentions]": search.mentions }),
          ...(search.has === undefined ? {} : { "filter[has]": [...search.has] }),
          ...(search.community === undefined ? {} : { "filter[community]": search.community }),
          ...(search.channel === undefined ? {} : { "filter[channel]": search.channel }),
          ...(search.before === undefined ? {} : { before: search.before }),
          limit: SEARCH_PAGE,
          include: ["authors", "attachments", "polls", "channels", "reactions"],
        },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ ...result.data.included, messages: result.data.data });
    this.store.setReactions(
      result.data.data.map((m) => m.id),
      result.data.included.reactions ?? [],
    );
    return result.data.data;
  }

  /**
   * Changes the caller's own profile: any of the display name, pronouns, bio, and status, with
   * `null` clearing one. The cache is left to the update event, which the server publishes
   * before it answers.
   */
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
        query: {
          include: [
            "channels",
            "categories",
            "members",
            "voice",
            "readStates",
            "mutes",
            "collapses",
            "roles",
            "notifications",
          ],
        },
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
      const [me, communities, dms, admin, blocks] = await Promise.all([
        this.#client.api.GET("/api/v1/users/{user}", { params: { path: { user: "@me" } } }),
        this.#client.api.GET("/api/v1/users/{user}/communities", {
          params: {
            path: { user: "@me" },
            query: {
              include: [
                "channels",
                "categories",
                "members",
                "voice",
                "readStates",
                "mutes",
                "collapses",
                "roles",
                "notifications",
              ],
            },
          },
        }),
        this.#client.api.GET("/api/v1/users/@me/dms", {
          params: { query: { include: ["users", "readStates", "mutes", "notifications"] } },
        }),
        this.#client.api.GET("/api/v1/users/@me/admin"),
        this.#client.api.GET("/api/v1/users/@me/blocks", {
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
      if (blocks.data === undefined) {
        throw new ApiProblemError(problemOf(blocks.error, blocks.response));
      }
      this.store.setBootstrap(me.data, communities.data.data, communities.data.included);
      this.store.ingest(dms.data.included);
      this.store.setDms(dms.data.data);
      this.store.replaceMutes([
        ...(communities.data.included.channelMutes ?? []),
        ...(dms.data.included.channelMutes ?? []),
      ]);
      this.#scheduleMuteEnd();
      this.store.replaceNotificationSettings([
        ...(communities.data.included.notificationSettings ?? []),
        ...(dms.data.included.notificationSettings ?? []),
      ]);
      this.store.ingest(blocks.data.included);
      this.store.replaceBlocks(blocks.data.data.map((block) => block.user));
      this.store.setDeploymentPermissions(admin.data?.permissions ?? []);
      this.store.replaceCollapsed(
        (communities.data.included.categoryCollapses ?? []).map((c) => c.category),
      );
      if (this.#ownsPreferences) {
        await this.preferences.loadAccount();
      }
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
    // Only what arrives live, and was posted since this sync began, is news: the replay a
    // connection starts with is not.
    if (event.serverEvent === "message" && event.type === "create") {
      const message = this.store.message(event.id);
      if (
        message !== undefined &&
        // Some seconds' allowance for the server's clock and this device's disagreeing.
        Date.parse(message.timestamp) >= this.#bootstrappedAt - NOTIFY_CLOCK_SLACK_MS &&
        this.store.notifies(message)
      ) {
        for (const listener of this.#notifyListeners) {
          listener(message);
        }
      }
    }
  }

  #apply(event: ServerEvent): void {
    if (event.serverEvent === "foreignDmJoined") {
      for (const listener of this.#foreignDmListeners) {
        listener({
          domain: event.domain,
          channel: event.channel,
          byName: event.byName,
          byDisplayName: event.byDisplayName ?? null,
        });
      }
      return;
    }
    // A deleted message that was a channel's newest leaves the store unable to say what is
    // newest now, so the channel's read state is read again.
    const orphaned =
      event.serverEvent === "message" && event.type === "delete"
        ? this.store.channelsLastMessaged(event.id)
        : [];
    const widens = this.#mayWidenAccess(event);
    const retagged = this.#unreadTagsChangedBy(event);
    const blockChanged =
      event.serverEvent === "userBlockChanged" && this.store.blocked(event.user) !== event.blocked;
    this.store.applyEvent(event);
    if (blockChanged) {
      this.#blockChanged();
    }
    for (const channelId of orphaned) {
      void this.#reloadReadState(channelId);
    }
    if (retagged !== null) {
      void this.#reloadReadState(retagged);
    }
    if (event.serverEvent === "channelRead") {
      this.#recountTags(event.channel);
    }
    if (widens !== null) {
      this.#scheduleAccessReload(widens);
    }
    if (event.serverEvent === "message" && event.type === "create") {
      this.ensureUser(event.author);
    }
    if (event.serverEvent === "userPreferencesChanged") {
      // Another of the user's devices changed something; the values are fetched rather than
      // carried by the event, so they never reach anyone else's stream.
      if (this.#ownsPreferences && event.user === this.store.me()?.id) {
        void this.preferences.loadAccount().catch(() => undefined);
      }
      return;
    }
    if (event.serverEvent === "voiceSessionEnded") {
      this.voice.onSessionEnded(event);
    }
    if (event.serverEvent === "channelMuteChanged") {
      this.#scheduleMuteEnd();
    }
    if (event.serverEvent === "react" && event.type === "delete") {
      // One of the few a summary names left; the next to have reacted is read again.
      const summary = this.store.reactions(event.messageId).get(event.emoji);
      if (
        summary !== undefined &&
        summary.users.length < Math.min(summary.count, REACTION_SUMMARY_USERS)
      ) {
        void this.#reloadReactions(event.messageId);
      }
    }
  }

  /**
   * The channel whose unread tags of the caller `event` may change without saying how: an
   * unread message's tags edited, or an unread message that tagged them deleted. `null` for
   * anything else; a new message's tags the store counts itself.
   */
  #unreadTagsChangedBy(event: ServerEvent): string | null {
    if (event.serverEvent !== "message" || event.type === "create") {
      return null;
    }
    const message = this.store.message(event.id);
    if (message === undefined) {
      return null;
    }
    const unread = message.id > (this.store.readState(message.channelId)?.lastRead ?? "");
    const changes =
      event.type === "update" ? event.mentions != null : this.store.mentionsMe(message);
    return unread && changes ? message.channelId : null;
  }

  /**
   * The community in which `event` may let the caller view channels they could not, which the
   * server then sends nothing about until they are read: a change to a role they hold (or
   * everyone's), to an override for one, to the roles they hold, or to who owns it. `null` for
   * anything else. Losing access needs no read; the store lets such channels go itself.
   */
  #mayWidenAccess(event: ServerEvent): string | null {
    const me = this.store.myUserId;
    const holds = (community: string, role: string): boolean =>
      this.store.roles(community).some((r) => r.id === role && r.everyone) ||
      (me !== null && (this.store.memberRoles(community, me) ?? []).includes(role));
    switch (event.serverEvent) {
      case "role": {
        if (event.type !== "update" || event.permissions == null) {
          return null;
        }
        const community = this.#communityOfRole(event.id);
        return community !== undefined && holds(community, event.id) ? community : null;
      }
      case "channelOverride": {
        const community = this.store.channel(event.channel)?.community;
        return community != null && holds(community, event.role) ? community : null;
      }
      case "categoryOverride": {
        const community = this.store.category(event.category)?.community;
        return community !== undefined && holds(community, event.role) ? community : null;
      }
      case "userCommunity":
        return event.type === "update" && event.user === me && event.roles != null
          ? event.community
          : null;
      case "community":
        return event.type === "update" && event.owner !== undefined && event.owner === me
          ? event.id
          : null;
      default:
        return null;
    }
  }

  #communityOfRole(roleId: string): string | undefined {
    for (const community of this.store.communities()) {
      if (this.store.roles(community.id).some((r) => r.id === roleId)) {
        return community.id;
      }
    }
    return undefined;
  }

  /**
   * Reads a community again soon, at a random moment within `ACCESS_RELOAD_SPREAD_MS` so that a
   * change reaching every member does not bring every member's read at once.
   */
  #scheduleAccessReload(communityId: string): void {
    if (this.#accessReloads.has(communityId)) {
      return;
    }
    this.#accessReloads.add(communityId);
    const generation = this.#generation;
    this.#setTimeout(() => {
      this.#accessReloads.delete(communityId);
      if (generation === this.#generation) {
        void this.loadCommunity(communityId).catch(() => undefined);
      }
    }, this.#random() * ACCESS_RELOAD_SPREAD_MS);
  }

  async #reloadReactions(messageId: string): Promise<void> {
    const generation = this.#generation;
    const result = await this.#client.api
      .GET("/api/v1/messages/{message}", {
        params: { path: { message: messageId }, query: { include: ["reactions"] } },
      })
      .catch(() => undefined);
    if (result?.data !== undefined && generation === this.#generation) {
      this.store.setReactions([messageId], result.data.included.reactions ?? []);
    }
  }

  /**
   * Ends each timed mute when its time comes, by this device's clock, since the server announces
   * no end it did not make; a device that slept past one ends it on waking, when the timer runs.
   */
  #scheduleMuteEnd(): void {
    if (this.#muteTimer !== null) {
      clearTimeout(this.#muteTimer);
      this.#muteTimer = null;
    }
    const next = this.store.nextMuteEnd();
    if (next === null) {
      return;
    }
    // A timer longer than this overflows and fires at once; a later end is waited for in steps.
    const delay = Math.min(Math.max(next - Date.now(), 0), MAX_TIMER_MS);
    this.#muteTimer = setTimeout(() => {
      this.#muteTimer = null;
      this.store.expireMutes(Date.now());
      this.#scheduleMuteEnd();
    }, delay);
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
        query: {
          ...query,
          include: ["authors", "attachments", "polls", "threads", "echoes", "reactions"],
        },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest(result.data.included);
    this.store.setReactions(
      result.data.data.map((m) => m.id),
      result.data.included.reactions ?? [],
    );
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
