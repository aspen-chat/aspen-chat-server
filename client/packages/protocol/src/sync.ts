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

import { AdminApi, adminRead } from "./admin";
import { EventStream, type EventStreamOptions } from "./events";
import type {
  CommunityPlugin,
  CustomEmoji,
  Message as EventMessage,
  ServerEvent,
} from "./generated/events";
import type { components } from "./generated/openapi";
import { type AspenClient, problemOf } from "./http";
import { ApiProblemError, type Problem, transportProblem } from "./problem";
import {
  AUDIO_INPUT,
  AUDIO_OUTPUT,
  TYPING_NOTICES,
  VIDEO_INPUT,
  PreferenceStore,
  effectiveStreamVolume,
  effectiveUserVolume,
  streamMuted,
  streamVolume,
  userMuted,
  userVolume,
  type PreferenceStorage,
} from "./preferences";
import type { OverrideGrant } from "./permissions";
import { REACTION_SUMMARY_USERS, RecordStore } from "./store";
import type { HeldMessage, Included, Invocation, NotificationLevel } from "./storeTypes";
import { lazyBrowserMedia, pageStorage } from "./platform";
import { type UploadTarget, describeAttachment, uploadAttachment, uploadIcon } from "./upload";
import { eventStreamUrl } from "./urls";
import { VoiceCall } from "./voice";
import type { VoiceMedia } from "./voiceMedia";

// The store's record, which the event stream carries; a read's record is assignable to it.
type Message = EventMessage;
type Attachment = components["schemas"]["Attachment"];
type Invite = components["schemas"]["Invite"];
type Community = components["schemas"]["Community"];
type Channel = components["schemas"]["Channel"];
type ChannelType = components["schemas"]["ChannelType"];
type Category = components["schemas"]["Category"];
type Poll = components["schemas"]["Poll"];
type PollCreateRequest = components["schemas"]["PollCreateRequest"];
type User = components["schemas"]["User"];
export type BotTransfer = components["schemas"]["BotTransfer"];
export type MessageHolding = components["schemas"]["MessageHolding"];

/** What became of a message sent: posted, or held by the server for its previews. */
export type Sent = { kind: "posted"; message: Message } | { kind: "held"; held: HeldMessage };

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

/** Which part of the activity feed to read (`AspenSync.readActivity`). */
export interface ActivityFilter {
  /** Only these communities' messages; every community's when absent. */
  communities?: readonly string[];
  /** Whether DMs' messages are read. */
  dms: boolean;
  /** Only messages the caller has not read. */
  unread: boolean;
  /** The last message of the previous page. */
  before?: string;
}

/** How many messages one page of the activity feed holds. */
export const ACTIVITY_PAGE = 25;
/** How many messages one page of saved messages holds. */
export const SAVED_PAGE = 50;

/** What a feed or saved messages read brings with each message, so it renders as in a channel. */
const LISTED_INCLUDES = [
  "authors",
  "memberships",
  "attachments",
  "polls",
  "channels",
  "reactions",
  "readStates",
] as const;

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

/** Messages fetched for a channel's latest window, which is read before anything shows. */
export const MESSAGE_PAGE_SIZE = 50;
/**
 * Messages fetched per page further back or forward through history, which is read ahead of
 * the reader. Fifty: a page's rows are committed to the DOM and laid out in one task, and at
 * fifty that task stays under 50ms on a phone, where a hundred took over 120ms and froze a
 * finger for as long; the list reads pages well ahead, so the smaller page costs no waiting.
 */
export const HISTORY_PAGE_SIZE = 50;
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

/**
 * How far before this sync began a message may say it was posted and still notify, allowing for
 * the server's clock and the device's disagreeing; well inside the minute a connection replays.
 */
const NOTIFY_CLOCK_SLACK_MS = 5_000;

/** How many people a page of a reaction list holds. */
export const REACTORS_PAGE = 50;

/** How many voters one read of an answer's voters asks for. */
export const VOTERS_PAGE = 50;

/** How many DMs one read of the DM list asks for, the most the server lists at once. */
export const DM_PAGE = 100;

/**
 * How many records a page of a moderator's or owner's list holds (bans, server mutes, invites,
 * held messages); a shorter page is the last.
 */
export const LIST_PAGE = 100;

/** The most channels the server sends typing for on one connection (`viewing`). */
const MAX_VIEWING = 8;

/** The longest delay `setTimeout` keeps; a longer one fires at once. */
const MAX_TIMER_MS = 2 ** 31 - 1;
/**
 * The least time between two activity reports; mirrors the event stream's `ACTIVITY_INTERVAL`,
 * which ignores reports closer together.
 */
export const ACTIVITY_INTERVAL_MS = 60_000;

/**
 * How often the server is told again that the user is still typing in a channel, while they
 * type; mirrors the server's `TYPING_REFRESH_SECONDS`.
 */
export const TYPING_REFRESH_MS = 3_000;
/**
 * How long someone is shown typing after the last word that they are; mirrors the server's
 * `TYPING_EXPIRY_SECONDS`.
 */
export const TYPING_EXPIRY_MS = 8_000;

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

/** A plugin's own event, for its views. */
export type PluginEvent = Extract<ServerEvent, { serverEvent: "pluginEvent" }>;
/** What a plugin tells the user of. */
export type PluginNotice = Extract<ServerEvent, { serverEvent: "pluginNotice" }>;

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

/**
 * How long a read of a community's member sample after a change to its roles shown apart may
 * wait, at most. Every member hears the change, so their reads are spread wide.
 */
export const MEMBER_RESAMPLE_SPREAD_MS = 10_000;

export class AspenSync {
  readonly store: RecordStore;
  /** The Administration Dashboard's calls. */
  readonly admin: AdminApi;
  /** The voice call, if any; a `VoiceCall` even when idle so the UI can subscribe once. */
  readonly voice: VoiceCall;
  /** The user's preferences, device-scoped and account-scoped alike. */
  readonly preferences: PreferenceStore;
  readonly #client: AspenClient;
  readonly #stream: EventStream;
  readonly #now: () => number;
  readonly #uploadFetch: typeof globalThis.fetch;
  /** Whether uploads go through a `fetch` of the caller's rather than the browser's own. */
  readonly #customUpload: boolean;
  readonly #listeners = new Set<SyncListener>();
  #status: SyncStatus = "stopped";
  #lastError: Problem | null = null;
  /** Events received while a bootstrap is in flight, applied once it lands. `null` when live. */
  #held: ServerEvent[] | null = null;
  #bootstrappedAt = 0;
  /** Increments on every start/stop so a stale async step can notice and bail. */
  #generation = 0;
  /** Whether a further page of the DM list is being read. */
  #loadingDms = false;
  readonly #windowLoads = new Map<string, Promise<void>>();
  /** Reads of what a message links to, under way, by the message linking. */
  readonly #linkLoads = new Map<string, Promise<void>>();
  /** The read of the plugin catalogue in flight, which every ask shares. */
  #pluginLoad: Promise<void> | null = null;
  readonly #userLoads = new Map<string, Promise<void>>();
  readonly #channelLoads = new Map<string, Promise<void>>();
  readonly #setTimeout: typeof globalThis.setTimeout;
  #presenceTimer: ReturnType<typeof setTimeout> | null = null;
  /** When the user last did something in the app, and when the server was last told. */
  #lastActivityAt = Number.NEGATIVE_INFINITY;
  #activityReportedAt = Number.NEGATIVE_INFINITY;
  /** The channels the server was told the user is typing in, and when it was last told. */
  readonly #typingSent = new Map<string, number>();
  /**
   * The channels open where someone typing is shown, each with how many places show it; the
   * server is told of them (`#tellViewing`), since it sends typing only where it is shown.
   */
  readonly #viewing = new Map<string, number>();
  /** When the next of those shown typing runs out. */
  #typingTimer: ReturnType<typeof setTimeout> | null = null;
  readonly #attachmentLoads = new Map<string, Promise<void>>();
  readonly #pollLoads = new Map<string, Promise<void>>();
  /** Per channel, the furthest message read and not yet reported. */
  readonly #unreported = new Map<string, string>();
  #readTimer: ReturnType<typeof setTimeout> | null = null;
  #muteTimer: ReturnType<typeof setTimeout> | null = null;
  readonly #iconLoads = new Map<string, Promise<void>>();
  readonly #random: () => number;
  /** Communities waiting to be read again because the caller's access in them may have grown. */
  readonly #accessReloads = new Set<string>();
  readonly #memberResamples = new Set<string>();
  /** Channels heard of but not held, waiting to be looked up (`#scheduleDiscovery`). */
  readonly #discoveries = new Set<string>();
  readonly #pinLoads = new Map<string, Promise<void>>();
  /** The channels whose online count is shown, with how many places show each. */
  readonly #presenceChannels = new Map<string, number>();
  readonly #commandLoads = new Map<string, Promise<void>>();
  readonly #frequentEmojiLoads = new Map<string, Promise<void>>();
  /** Reads of a channel's newest page under way, which a second ask joins. */
  readonly #latestLoads = new Map<string, Promise<void>>();
  readonly #foreignDmListeners = new Set<(notice: ForeignDmNotice) => void>();
  readonly #notifyListeners = new Set<(message: Message) => void>();
  readonly #pluginEventListeners = new Set<(event: PluginEvent) => void>();
  readonly #pluginNoticeListeners = new Set<(notice: PluginNotice) => void>();
  /** Whether `preferences` is this sync's own, loaded from and cleared with its server. */
  readonly #ownsPreferences: boolean;

  constructor(options: AspenSyncOptions) {
    this.#client = options.client;
    this.admin = new AdminApi(options.client);
    this.store = options.store ?? new RecordStore({ now: options.now ?? (() => Date.now()) });
    this.#now = options.now ?? (() => Date.now());
    this.#uploadFetch = options.uploadFetch ?? ((input, init) => globalThis.fetch(input, init));
    this.#customUpload = options.uploadFetch !== undefined;
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
    voiceOptions.userVolume = (userId, source) =>
      source === "screenAudio" ? this.#streamGain(userId) : this.#userGain(userId);
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
          camera: this.preferences.get(VIDEO_INPUT),
        })
        .catch(() => undefined);
    };
    this.preferences.subscribe(applyDevices);
    applyDevices();
    // Turning typing notices off says at once that the user stopped wherever they were typing.
    this.preferences.subscribe(() => {
      if (!this.preferences.get(TYPING_NOTICES)) {
        for (const channelId of Array.from(this.#typingSent.keys())) {
          this.stopTyping(channelId);
        }
      }
    });
    const streamOptions: EventStreamOptions = {
      url: eventStreamUrl(options.client.baseUrl),
      authenticate: (o) => options.client.freshSessionToken(o),
      onReady: (info) => {
        this.#onReady(info.resumed);
      },
      onEvent: (event) => {
        this.#onEvent(event);
      },
      onEphemeral: (event) => {
        // A newer server may tell of what this client does not know, which changes nothing.
        const kind: string = event.type;
        if (kind !== "typing") {
          return;
        }
        this.store.noteTyping(
          event.channelId,
          event.userId,
          event.typing ? this.#now() + TYPING_EXPIRY_MS : null,
        );
        this.#expireTyping();
      },
      onResyncRequired: () => {
        void this.#resync();
      },
      onConnectionLost: () => {
        // Nobody will say when those shown typing stop, and the server let go of what this
        // connection said.
        this.#forgetTyping();
        if (this.#status === "live" || this.#status === "connecting") {
          this.#setStatus("reconnecting");
        }
      },
      onEnrollmentRequired: () => {
        options.client.noticeEnrollmentRequired();
      },
      onVerificationRequired: () => {
        options.client.noticeVerificationRequired();
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

  /**
   * Registers for plugins' own events (`pluginEvent`), which only their views use, and returns
   * the unsubscribe function.
   */
  readonly onPluginEvent = (listener: (event: PluginEvent) => void): (() => void) => {
    this.#pluginEventListeners.add(listener);
    return () => {
      this.#pluginEventListeners.delete(listener);
    };
  };

  /**
   * Registers for what plugins tell the user of (`pluginNotice`), which the server sends only
   * where their settings would tell them of a message that tags them, and returns the
   * unsubscribe function. How to tell them is the app's.
   */
  readonly onPluginNotice = (listener: (notice: PluginNotice) => void): (() => void) => {
    this.#pluginNoticeListeners.add(listener);
    return () => {
      this.#pluginNoticeListeners.delete(listener);
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
    this.#forgetTyping();
    this.#held = null;
    this.#windowLoads.clear();
    this.#userLoads.clear();
    this.#channelLoads.clear();
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
    const latest = this.#latestLoads.get(channelId);
    if (latest !== undefined) {
      return latest;
    }
    // A page already on its way may be one that does not reach the latest; this follows it.
    const pending = this.#windowLoads.get(channelId);
    const load = (pending ?? Promise.resolve())
      .catch(() => undefined)
      .then(() =>
        this.#loadWindow(channelId, async () => {
          const messages = await this.#readMessages(channelId, { limit: MESSAGE_PAGE_SIZE });
          this.store.replaceWindow(channelId, messages, {
            hasOlder: messages.length === MESSAGE_PAGE_SIZE,
            atLatest: true,
          });
        }),
      )
      .finally(() => {
        if (this.#latestLoads.get(channelId) === load) {
          this.#latestLoads.delete(channelId);
        }
      });
    this.#latestLoads.set(channelId, load);
    return load;
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
        limit: HISTORY_PAGE_SIZE,
      });
      this.store.prependWindow(channelId, messages, messages.length === HISTORY_PAGE_SIZE);
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
        limit: HISTORY_PAGE_SIZE,
      });
      this.store.appendWindow(channelId, messages, messages.length < HISTORY_PAGE_SIZE);
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
  uploadAttachment(
    file: File,
    /** A picture's size in pixels, which readers use to make room for it before it loads. */
    size?: { readonly width: number; readonly height: number },
    /** Told how many of the file's bytes have reached storage, as they go. */
    onProgress?: (sent: number, total: number) => void,
  ): Promise<Attachment> {
    return uploadAttachment(this.#uploadTarget(), file, size, onProgress);
  }

  /**
   * Sets or clears (with `null`) what an attachment not yet sent shows, which readers' apps give
   * as its text alternative, and caches the record as it now is.
   */
  describeAttachment(attachmentId: string, description: string | null): Promise<Attachment> {
    return describeAttachment(this.#uploadTarget(), attachmentId, description);
  }

  /**
   * Uploads an icon in the server's two phases, reserving it, sending the bytes straight to
   * storage, and confirming, and caches the record. The caller then names the icon on a user
   * or community.
   */
  uploadIcon(bytes: Blob, mimeType: string): Promise<Icon> {
    return uploadIcon(this.#uploadTarget(), bytes, mimeType);
  }

  #uploadTarget(): UploadTarget {
    return {
      client: this.#client,
      store: this.store,
      uploadFetch: this.#uploadFetch,
      customUpload: this.#customUpload,
    };
  }

  /** Fetches an icon record the cache lacks, once, for a user or community that names it. */
  ensureIcon(iconId: string): void {
    if (
      this.store.icon(iconId) !== undefined ||
      this.store.missing("icon", iconId) ||
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
          this.store.markMissing("icon", iconId);
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
      this.store.missing("attachment", attachmentId) ||
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
          this.store.markMissing("attachment", attachmentId);
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
   *
   * The server may hold a message while a preview of one of its attachments is being made, up to
   * twenty seconds from the upload, and post it then: the held message is kept in the store
   * (`heldMessages`) and shown waiting until `heldMessagePosted` or `heldMessageFailed`.
   */
  async sendMessage(
    channelId: string,
    content: string,
    attachments: readonly string[] = [],
    options: { echoToParent?: boolean } = {},
  ): Promise<Sent> {
    const result = await this.#client.api.POST("/api/v1/channels/{channel}/messages", {
      params: { path: { channel: channelId } },
      body: {
        content,
        attachments: [...attachments],
        ...(options.echoToParent === true ? { echoToParent: true } : {}),
        mayHold: true,
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    if (result.response.status === 202) {
      const held = result.data as HeldMessage;
      this.store.putHeldMessage(held);
      return { kind: "held", held };
    }
    const message = result.data as Message;
    this.store.addMessage(message);
    return { kind: "posted", message };
  }

  /**
   * Sends a held message the server dropped once more, as it was written, and lets the dropped
   * one go once the server has it. A first reply whose thread went with it makes the thread
   * anew.
   */
  async sendHeldAgain(heldId: string): Promise<Sent | undefined> {
    const entry = this.store.heldMessage(heldId);
    if (entry === undefined) {
      return undefined;
    }
    const { channelId, content, attachments, echoToParent } = entry.message;
    const sent =
      entry.startsThreadOf === null
        ? await this.sendMessage(channelId, content, attachments, { echoToParent })
        : await this.replyInThread(entry.startsThreadOf, content, attachments, { echoToParent });
    this.store.forgetHeldMessage(heldId);
    return sent;
  }

  /**
   * Posts a reply to the thread a message starts, which the server makes with its first reply,
   * held or posted as `sendMessage` says; the reply's `channelId` names the thread. The message
   * learns its thread at once, as the events that follow would say.
   */
  async replyInThread(
    starterId: string,
    content: string,
    attachments: readonly string[] = [],
    options: { echoToParent?: boolean } = {},
  ): Promise<Sent> {
    const result = await this.#client.api.POST("/api/v1/messages/{message}/thread/messages", {
      params: { path: { message: starterId } },
      body: {
        content,
        attachments: [...attachments],
        ...(options.echoToParent === true ? { echoToParent: true } : {}),
        mayHold: true,
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    let sent: Sent;
    if (result.response.status === 202) {
      const held = result.data as HeldMessage;
      this.store.putHeldMessage(held);
      sent = { kind: "held", held };
    } else {
      const message = result.data as Message;
      this.store.addMessage(message);
      sent = { kind: "posted", message };
    }
    this.store.applyEvent({
      serverEvent: "message",
      type: "update",
      id: starterId,
      thread: sent.kind === "held" ? sent.held.channelId : sent.message.channelId,
    });
    return sent;
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
   * Shows a thread reply that was posted without an echo in the thread's parent channel, which
   * only its author may do. The echo is cached at once and the reply learns its echo, as the
   * events that follow would say.
   */
  async echoReply(messageId: string): Promise<Message> {
    const result = await this.#client.api.PUT("/api/v1/messages/{message}/echo", {
      params: { path: { message: messageId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const echo = result.data;
    this.store.addMessage(echo);
    this.store.applyEvent({
      serverEvent: "message",
      type: "update",
      id: messageId,
      echo: echo.id,
    });
    return echo;
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
   * Reads one channel into the store in the background when it is not there and has not been
   * refused, as a link naming a DM from before the last listing does to show its people. A
   * channel the caller may not see, or that is gone, is marked missing.
   */
  ensureChannel(channelId: string): void {
    if (
      this.store.channel(channelId) !== undefined ||
      this.store.missing("channel", channelId) ||
      this.#channelLoads.has(channelId)
    ) {
      return;
    }
    const load = this.#client.api
      .GET("/api/v1/channels/{channel}", { params: { path: { channel: channelId } } })
      .then(({ data, response }) => {
        if (data !== undefined) {
          this.store.ingest({ channels: [data] });
        } else if (response.status === 403 || response.status === 404) {
          this.store.markMissing("channel", channelId);
        }
      })
      .catch(() => {
        // Transient; the next render that needs the channel asks again.
      })
      .finally(() => {
        this.#channelLoads.delete(channelId);
      });
    this.#channelLoads.set(channelId, load);
  }

  /**
   * Reads one message into the store when it is not there yet, such as the message a thread
   * opened from a link started. Its author, attachments, poll, and thread come with it. A
   * message the caller may not read, or that is gone, is marked missing.
   */
  async loadMessage(messageId: string): Promise<Message> {
    const held = this.store.message(messageId);
    if (held !== undefined) {
      return held;
    }
    const result = await this.#client.api.GET("/api/v1/messages/{message}", {
      params: {
        path: { message: messageId },
        query: {
          include: [
            "authors",
            "memberships",
            "attachments",
            "polls",
            "threads",
            "reactions",
            "linked",
            "warnings",
            "annotations",
          ],
        },
      },
    });
    if (result.data === undefined) {
      if (result.response.status === 403 || result.response.status === 404) {
        this.store.markMissing("message", messageId);
      }
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.#ingestMessageRead(result.data.included, [result.data.data]);
    this.store.setReactions([result.data.data.id], result.data.included.reactions ?? []);
    this.store.setAnnotations([result.data.data.id], result.data.included.messageAnnotations ?? []);
    return result.data.data;
  }

  /**
   * Reads the plugins the deployment runs again, as when an event names one this client does
   * not know of. Several asks share a read.
   */
  loadPlugins(): Promise<void> {
    if (this.#pluginLoad !== null) {
      return this.#pluginLoad;
    }
    this.#pluginLoad = this.#client.api
      .GET("/api/v1/plugins")
      .then((result) => {
        if (result.data === undefined) {
          throw new ApiProblemError(problemOf(result.error, result.response));
        }
        this.store.setPlugins(result.data);
      })
      .finally(() => {
        this.#pluginLoad = null;
      });
    return this.#pluginLoad;
  }

  /** Whether `event` names a plugin the catalogue does not hold. */
  #namesUnknownPlugin(event: ServerEvent): boolean {
    const unknown = (id: string) => this.store.plugin(id) === undefined;
    switch (event.serverEvent) {
      case "messageAnnotation":
      case "userAnnotation":
        return event.type === "create" && unknown(event.plugin);
      case "message":
        return (
          (event.type === "create" || event.type === "update") &&
          (event.alteredBy ?? []).some(unknown)
        );
      default:
        return false;
    }
  }

  /**
   * Presses a button of a message's card, which calls its plugin as the user. Answers the
   * status, which is the plugin's to choose, so it says only whether the press worked.
   */
  pressCardButton(messageId: string, button: string): Promise<number> {
    return this.#client.pressCardButton(messageId, button);
  }

  /** Calls a plugin's route as the user, for a plugin's view. */
  pluginRoute(
    plugin: string,
    request: { method: string; path: string; query?: string; body?: string },
  ): Promise<{ status: number; contentType: string | null; body: string }> {
    return this.#client.pluginRoute(plugin, request);
  }

  /** Where the deployment's API is, for the URLs plugins' views are shown from. */
  get apiBase(): string {
    return this.#client.baseUrl;
  }

  /** Reads what plugins say about a person. */
  async loadUserAnnotations(userId: string): Promise<void> {
    const result = await this.#client.api.GET("/api/v1/users/{user}/annotations", {
      params: { path: { user: userId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.setUserAnnotations(userId, result.data);
  }

  /** Reads a community's use of each plugin, which takes Manage plugins. */
  async loadCommunityPlugins(communityId: string): Promise<void> {
    const result = await this.#client.api.GET("/api/v1/communities/{community}/plugins", {
      params: { path: { community: communityId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.setCommunityPlugins(communityId, result.data);
  }

  /**
   * Turns a plugin on in a community, with `settings` laid over any it had, bringing in its
   * account holding `grant`.
   */
  async enableCommunityPlugin(
    communityId: string,
    pluginId: string,
    settings: Record<string, unknown>,
    grant: readonly Permission[],
  ): Promise<CommunityPlugin> {
    const result = await this.#client.api.PUT("/api/v1/communities/{community}/plugins/{plugin}", {
      params: { path: { community: communityId, plugin: pluginId } },
      body: { settings, grant: [...grant] },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.putCommunityPlugin(result.data);
    return result.data;
  }

  /** Changes a plugin's settings in a community; `null` restores a setting's default. */
  async configureCommunityPlugin(
    communityId: string,
    pluginId: string,
    patch: Record<string, unknown>,
  ): Promise<CommunityPlugin> {
    const result = await this.#client.api.PATCH(
      "/api/v1/communities/{community}/plugins/{plugin}",
      {
        params: { path: { community: communityId, plugin: pluginId } },
        body: patch,
      },
    );
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.putCommunityPlugin(result.data);
    return result.data;
  }

  /** Turns a plugin off in a community, taking its account out. */
  async disableCommunityPlugin(communityId: string, pluginId: string): Promise<void> {
    const result = await this.#client.api.DELETE(
      "/api/v1/communities/{community}/plugins/{plugin}",
      { params: { path: { community: communityId, plugin: pluginId } } },
    );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const held = this.store.communityPlugins(communityId)?.find((p) => p.plugin === pluginId);
    if (held !== undefined) {
      this.store.putCommunityPlugin({ ...held, enabled: false });
    }
  }

  /**
   * Reads what a held message links to and, for a warning, what it is about, as this caller
   * finds them: for a message that arrived live or was edited, whose links no read has brought.
   * Several asks for one message share a read.
   */
  loadLinks(messageId: string): Promise<void> {
    const pending = this.#linkLoads.get(messageId);
    if (pending !== undefined) {
      return pending;
    }
    const promise = this.#client.api
      .GET("/api/v1/messages/{message}", {
        params: {
          path: { message: messageId },
          query: { include: ["linked", "warnings", "authors", "memberships", "attachments"] },
        },
      })
      .then((result) => {
        if (result.data === undefined) {
          throw new ApiProblemError(problemOf(result.error, result.response));
        }
        this.#ingestMessageRead(result.data.included);
      })
      .finally(() => {
        this.#linkLoads.delete(messageId);
      });
    this.#linkLoads.set(messageId, promise);
    return promise;
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
   * when given. The result is cached at once; the matching event is then a no-op. The overrides
   * it starts with are applied as their events would be, so a channel its creator has shut
   * themselves out of leaves the store again at once, whichever of the response and the events
   * arrives first.
   */
  async createChannel(
    communityId: string,
    options: {
      name: string;
      ty: ChannelType;
      parentCategory: string | null;
      overrides?: readonly OverrideGrant[];
      /** For a channel of `ty` `plugin`, the kind a plugin adds. */
      pluginType?: string;
    },
  ): Promise<Channel> {
    const sortIndex = this.store
      .channels(communityId)
      .reduce((max, channel) => Math.max(max, channel.sortIndex + 1), 0);
    const overrides = options.overrides ?? [];
    const result = await this.#client.api.POST("/api/v1/channels", {
      body: {
        name: options.name,
        ty: options.ty,
        community: communityId,
        parentCategory: options.parentCategory,
        sortIndex,
        ...(options.pluginType === undefined ? {} : { pluginType: options.pluginType }),
        ...(overrides.length > 0
          ? {
              overrides: overrides.map((o) => ({
                role: o.role,
                allow: [...o.allow],
                deny: [...o.deny],
              })),
            }
          : {}),
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const channel = result.data;
    this.store.ingest({ channels: [channel] });
    for (const o of overrides) {
      this.store.applyEvent({
        serverEvent: "channelOverride",
        type: "create",
        channel: channel.id,
        role: o.role,
        allow: [...o.allow],
        deny: [...o.deny],
      });
    }
    return channel;
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

  /**
   * Everyone who voted for one answer of a poll, earliest first, a page at a time: the page
   * after `after`, or the first. A page shorter than `VOTERS_PAGE` is the last. The users are
   * stored as they come.
   */
  async loadVoters(pollId: string, option: number, after?: string): Promise<User[]> {
    const result = await this.#client.api.GET("/api/v1/polls/{poll}/votes/{option}", {
      params: {
        path: { poll: pollId, option },
        query: after === undefined ? { limit: VOTERS_PAGE } : { after, limit: VOTERS_PAGE },
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

  /**
   * Records that the caller has seen `messageId` in `channelId`, a channel, DM, or thread: at
   * once in the store, and to the server within `READ_REPORT_MS`, together with whatever else
   * was read meanwhile. Reading behind the current position changes nothing.
   */
  markRead(channelId: string, messageId: string): void {
    if (this.store.channel(channelId)?.ty === "thread") {
      const read = this.store.threadRead(channelId);
      if (read !== undefined && messageId <= read) {
        return;
      }
      this.store.setThreadRead(channelId, messageId);
    } else {
      const state = this.store.readState(channelId);
      if (state === undefined || messageId <= state.lastRead) {
        return;
      }
      this.store.setLastRead(channelId, messageId);
    }
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

  /**
   * Someone's DMs, for a deployment moderator to open; they are put in the store so the DM
   * screen can show one, and reading any is logged by the server.
   */
  async userDms(userId: string): Promise<Channel[]> {
    const dms = adminRead(
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

  /** Removes a written-in answer, and every vote for it. */
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
      this.store.missing("poll", pollId) ||
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
          this.store.markMissing("poll", pollId);
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
   * How loud the sound of what `userId` shares is to this user on this install, set apart from
   * their voice: a gain, 1 as sent.
   */
  async setStreamVolume(userId: string, gain: number): Promise<void> {
    await this.preferences.set(streamVolume(userId), gain);
    this.voice.setUserVolume(userId, this.#streamGain(userId), "screenAudio");
  }

  /** Silences the sound of what `userId` shares for this user alone, their voice apart. */
  async setStreamMuted(userId: string, muted: boolean): Promise<void> {
    await this.preferences.set(streamMuted(userId), muted);
    this.voice.setUserVolume(userId, this.#streamGain(userId), "screenAudio");
  }

  /** How loud `userId`'s stream plays here: silent while muted for this user, or blocked. */
  #streamGain(userId: string): number {
    return this.store.silenced(userId) ? 0 : effectiveStreamVolume(this.preferences, userId);
  }

  /**
   * Who the user blocked on every deployment they use, by `identityOf`, with `domain`, this
   * deployment's name, and `home`, their home's: those of them in a call here are silenced and
   * their screens hidden, as if blocked here.
   */
  setBlockedIdentities(domain: string, home: string | null, identities: ReadonlySet<string>): void {
    this.store.setBlockedIdentities(domain, home, identities);
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

  /**
   * Reads every bot the caller owns into the store (`RecordStore.ownedBots`). A bot the store
   * still holds as theirs that the answer lacks (accepted by someone it was offered to, whose
   * owner change reaches only those sharing a community with it) is read again, so its new
   * owner replaces the stale one.
   */
  async loadBots(): Promise<void> {
    const result = await this.#client.api.GET("/api/v1/users/@me/bots");
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const owned = new Set(result.data.map((bot) => bot.id));
    const stale = this.store.ownedBots().filter((bot) => !owned.has(bot.id));
    this.store.ingest({ users: result.data });
    const reread = await Promise.all(
      stale.map((bot) =>
        this.#client.api.GET("/api/v1/users/{user}", { params: { path: { user: bot.id } } }),
      ),
    );
    const found = reread.flatMap((answer) => (answer.data === undefined ? [] : [answer.data]));
    this.store.ingest({ users: found });
    for (const [index, answer] of reread.entries()) {
      const bot = stale[index];
      if (answer.response.status === 404 && bot !== undefined) {
        this.store.forgetUser(bot.id);
      }
    }
  }

  /**
   * Makes a bot the caller owns. Resolves to the bot and its token, which the server shows only
   * this once.
   */
  async createBot(name: string, displayName: string | null): Promise<{ bot: User; token: string }> {
    const result = await this.#client.api.POST("/api/v1/users/@me/bots", {
      // Composed (NFC), as the server requires a new name to be.
      body: { name: name.normalize("NFC"), displayName },
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

  /** Changes the profile of a bot the caller owns, as `updateProfile` changes their own. */
  async updateBotProfile(botId: string, patch: UserUpdateRequest): Promise<void> {
    const result = await this.#client.api.PATCH("/api/v1/users/{user}", {
      params: { path: { user: botId } },
      body: patch,
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    // The bot's own update event reaches only those who share a community with it.
    this.store.ingest({ users: [result.data] });
  }

  /** The offers of bots made to the caller or by them that still stand, the newest first. */
  async loadBotTransfers(): Promise<BotTransfer[]> {
    const result = await this.#client.api.GET("/api/v1/users/@me/bot-transfers");
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ users: result.data.map((transfer) => transfer.bot) });
    return result.data;
  }

  /**
   * Offers a bot the caller owns to someone else, who owns it once they accept; offering it
   * again replaces the offer. Needs a recently verified sign-in (`reauthenticationRequired`).
   */
  async offerBotTransfer(botId: string, ownerId: string): Promise<BotTransfer> {
    const result = await this.#client.api.PUT("/api/v1/bots/{bot}/transfer", {
      params: { path: { bot: botId } },
      body: { owner: ownerId },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    return result.data;
  }

  /** Withdraws the offer of a bot the caller made, or declines one made to them. */
  async endBotTransfer(botId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/bots/{bot}/transfer", {
      params: { path: { bot: botId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Accepts a bot offered to the caller: it becomes theirs, with a new token the server shows
   * only this once, and the old token stops working.
   */
  async acceptBotTransfer(botId: string): Promise<{ bot: User; token: string }> {
    const result = await this.#client.api.POST("/api/v1/bots/{bot}/transfer/acceptance", {
      params: { path: { bot: botId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ users: [result.data.bot] });
    return result.data;
  }

  /** The categories a report may be made in, in the order they are offered. */
  async reportCategories(): Promise<components["schemas"]["ReportCategory"][]> {
    const result = await this.#client.api.GET("/api/v1/report-categories");
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    return result.data;
  }

  /** Reports a message to the deployment's moderators. */
  async reportMessage(
    messageId: string,
    category: string,
    explanation: string | null,
  ): Promise<void> {
    const result = await this.#client.api.POST("/api/v1/messages/{message}/reports", {
      params: { path: { message: messageId } },
      body: { category, ...(explanation === null ? {} : { explanation }) },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Reports someone's profile to the deployment's moderators, naming what is wrong with it. */
  async reportProfile(
    userId: string,
    category: string,
    explanation: string | null,
    aspects: readonly components["schemas"]["ProfileAspect"][],
  ): Promise<void> {
    const result = await this.#client.api.POST("/api/v1/users/{user}/reports", {
      params: { path: { user: userId } },
      body: {
        category,
        aspects: [...aspects],
        ...(explanation === null ? {} : { explanation }),
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Reports the nickname a fellow member chose in a community to the deployment's moderators;
   * the server keeps it as it stands.
   */
  async reportNickname(
    communityId: string,
    userId: string,
    category: string,
    explanation: string | null,
  ): Promise<void> {
    const result = await this.#client.api.POST(
      "/api/v1/communities/{community}/members/{user}/nickname/reports",
      {
        params: { path: { community: communityId, user: userId } },
        body: { category, ...(explanation === null ? {} : { explanation }) },
      },
    );
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Deletes a bot: the caller's own, or, with Manage deployment settings, one whose owner is gone. */
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
   * Reads the next page of the caller's DMs, after the last listed, when the server has more;
   * its mutes and notification settings already came whole with the first.
   */
  async loadMoreDms(): Promise<void> {
    if (this.store.dmsComplete() || this.#loadingDms) {
      return;
    }
    const before = this.store.lastListedDm();
    if (before === undefined) {
      return;
    }
    const generation = this.#generation;
    this.#loadingDms = true;
    try {
      const page = await this.#client.api.GET("/api/v1/users/@me/dms", {
        params: { query: { include: ["users", "readStates", "voice"], before, limit: DM_PAGE } },
      });
      if (generation !== this.#generation || page.data === undefined) {
        return;
      }
      this.store.ingest(page.data.included);
      this.store.appendDms(page.data.data, page.data.data.length < DM_PAGE);
    } finally {
      this.#loadingDms = false;
    }
  }

  /**
   * Every message of the caller's held for its previews, a page at a time; they are few, since
   * each is posted within seconds. A page that cannot be read ends the reading with what came.
   */
  async #readHeldMessages(): Promise<HeldMessage[]> {
    const held: HeldMessage[] = [];
    for (;;) {
      const after = held.at(-1)?.id;
      const page = await this.#client.api.GET("/api/v1/users/@me/held-messages", {
        params: { query: after === undefined ? { limit: LIST_PAGE } : { after, limit: LIST_PAGE } },
      });
      if (page.data === undefined) {
        return held;
      }
      held.push(...page.data);
      if (page.data.length < LIST_PAGE) {
        return held;
      }
    }
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

  /**
   * Declines the DM call ringing the caller: it stops ringing them on every device, which their
   * ring's `delete` event tells.
   */
  async declineCall(channelId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/channels/{channel}/voice/rings/@me", {
      params: { path: { channel: channelId } },
    });
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

  /**
   * Closes a poll before its deadline, as its creator or a holder of Manage messages; the
   * final tally and the announcement come as events, and the answer is applied meanwhile.
   */
  async closePoll(pollId: string): Promise<void> {
    const result = await this.#client.api.POST("/api/v1/polls/{poll}/close", {
      params: { path: { poll: pollId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({ serverEvent: "poll", type: "update", ...result.data });
  }

  /**
   * Reads the first page of a community's standing bans into the store, for a holder of Ban
   * members, or, with `more`, the page after those held.
   */
  async loadBans(communityId: string, more = false): Promise<void> {
    const before = more ? this.store.bans(communityId)?.at(-1)?.user : undefined;
    if (more && (before === undefined || this.store.listComplete(`bans:${communityId}`))) {
      return;
    }
    const result = await this.#client.api.GET("/api/v1/communities/{community}/bans", {
      params: {
        path: { community: communityId },
        query: before === undefined ? { limit: LIST_PAGE } : { before, limit: LIST_PAGE },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const complete = result.data.length < LIST_PAGE;
    if (more) {
      this.store.appendBans(communityId, result.data, complete);
    } else {
      this.store.replaceBans(communityId, result.data, complete);
    }
  }

  /**
   * Reads the first page of a community's standing server mutes into the store, for a holder
   * of Manage calls, or, with `more`, the page after those held.
   */
  async loadVoiceMutes(communityId: string, more = false): Promise<void> {
    const before = more ? this.store.voiceMutes(communityId)?.at(-1)?.user : undefined;
    if (more && (before === undefined || this.store.listComplete(`voiceMutes:${communityId}`))) {
      return;
    }
    const result = await this.#client.api.GET("/api/v1/communities/{community}/voice-mutes", {
      params: {
        path: { community: communityId },
        query: before === undefined ? { limit: LIST_PAGE } : { before, limit: LIST_PAGE },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const complete = result.data.length < LIST_PAGE;
    if (more) {
      this.store.appendVoiceMutes(communityId, result.data, complete);
    } else {
      this.store.replaceVoiceMutes(communityId, result.data, complete);
    }
  }

  /** Reads whether a moderator's mute of one person stands, for a holder of Manage calls. */
  async loadVoiceMute(communityId: string, userId: string): Promise<void> {
    const result = await this.#client.api.GET(
      "/api/v1/communities/{community}/voice-mutes/{user}",
      { params: { path: { community: communityId, user: userId } } },
    );
    if (result.data !== undefined) {
      this.store.setVoiceMuted(communityId, userId, true);
    } else if (result.response.status === 404) {
      this.store.setVoiceMuted(communityId, userId, false);
    } else {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Lifts someone's server mute in a community; its deletion event changes the cache. */
  async liftVoiceMute(communityId: string, userId: string): Promise<void> {
    const result = await this.#client.api.DELETE(
      "/api/v1/communities/{community}/voice-mutes/{user}",
      { params: { path: { community: communityId, user: userId } } },
    );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({
      serverEvent: "voiceMute",
      type: "delete",
      community: communityId,
      user: userId,
    });
  }

  /**
   * Bans someone from a community, with a reason, for a time, and deleting their recent
   * messages where asked; the answer is applied when its event has not come first. Returns
   * how many messages the ban deleted.
   */
  async banMember(
    communityId: string,
    userId: string,
    request: { reason?: string; durationSeconds?: number; deleteMessagesSeconds?: number },
  ): Promise<number> {
    const result = await this.#client.api.PUT("/api/v1/communities/{community}/bans/{user}", {
      params: { path: { community: communityId, user: userId } },
      body: request,
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({ serverEvent: "communityBan", type: "create", ...result.data.ban });
    return result.data.deletedMessages;
  }

  /** Lifts a ban; its deletion event changes the cache. */
  async liftBan(communityId: string, userId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/communities/{community}/bans/{user}", {
      params: { path: { community: communityId, user: userId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({
      serverEvent: "communityBan",
      type: "delete",
      community: communityId,
      user: userId,
    });
  }

  /**
   * Adds a custom emoji to a community: uploads its picture as an icon, then names it. The
   * answer is applied when its event has not come first.
   */
  async createCustomEmoji(
    communityId: string,
    name: string,
    picture: Blob,
    mimeType: string,
  ): Promise<CustomEmoji> {
    const icon = await this.uploadIcon(picture, mimeType);
    const result = await this.#client.api.POST("/api/v1/communities/{community}/emoji", {
      params: { path: { community: communityId } },
      body: { name, icon: icon.id },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    if (this.store.customEmojiById(result.data.id) === undefined) {
      this.store.applyEvent({ serverEvent: "customEmoji", type: "create", ...result.data });
    }
    return result.data;
  }

  /** Renames a custom emoji; its update event changes the cache. */
  async renameCustomEmoji(emojiId: string, name: string): Promise<void> {
    const result = await this.#client.api.PATCH("/api/v1/emoji/{emoji}", {
      params: { path: { emoji: emojiId } },
      body: { name },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Removes a custom emoji; its deletion event changes the cache. */
  async deleteCustomEmoji(emojiId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/emoji/{emoji}", {
      params: { path: { emoji: emojiId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Changes a role as a merge patch: what `patch` leaves out is unchanged, and a `hue` of `null`
   * takes the role's colour away. Its update event changes the cache.
   */
  async updateRole(
    roleId: string,
    patch: {
      name?: string;
      permissions?: readonly Permission[];
      hue?: number | null;
      hoist?: boolean;
    },
  ): Promise<void> {
    const result = await this.#client.api.PATCH("/api/v1/roles/{role}", {
      params: { path: { role: roleId } },
      body: {
        ...(patch.name !== undefined ? { name: patch.name } : {}),
        ...(patch.permissions !== undefined ? { permissions: [...patch.permissions] } : {}),
        ...(patch.hue !== undefined ? { hue: patch.hue } : {}),
        ...(patch.hoist !== undefined ? { hoist: patch.hoist } : {}),
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
   * A page of a community's members whose name contains `name`, sorted by name, after the
   * member `after` when given. Only those who act on members may search a community larger
   * than its member sample; the server refuses anyone else. The members are cached, their roles
   * too, but not added to the sample.
   */
  async searchMembers(communityId: string, name: string, after?: string): Promise<User[]> {
    const result = await this.#client.api.GET("/api/v1/communities/{community}/members", {
      params: {
        path: { community: communityId },
        query:
          after === undefined
            ? { "filter[name]": name, limit: MEMBER_SEARCH_PAGE }
            : { "filter[name]": name, after, limit: MEMBER_SEARCH_PAGE },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({ users: result.data.data });
    this.store.noteMemberships(result.data.included.userCommunities ?? []);
    return result.data.data;
  }

  /** Reads a community's member sample again, replacing the one the store holds. */
  async loadMemberSample(communityId: string): Promise<void> {
    const result = await this.#client.api.GET("/api/v1/communities/{community}/members", {
      params: { path: { community: communityId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.ingest({
      users: result.data.data,
      userCommunities: result.data.included.userCommunities ?? [],
    });
  }

  /**
   * Reads one member's roles in a community into the cache, for a member outside the sample, and
   * answers whether they are a member at all.
   */
  async loadMember(communityId: string, userId: string): Promise<boolean> {
    const result = await this.#client.api.GET("/api/v1/communities/{community}/members/{user}", {
      params: { path: { community: communityId, user: userId } },
    });
    if (result.response.status === 404) {
      return false;
    }
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.noteMemberships([result.data]);
    return true;
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

  /**
   * Sets the caller's nickname in a community, or clears it with `null`. Setting one takes
   * Change nickname. The change arrives as the membership's update event.
   */
  async setNickname(communityId: string, nickname: string | null): Promise<void> {
    const result = await this.#client.api.PATCH("/api/v1/communities/{community}/members/@me", {
      params: { path: { community: communityId } },
      body: { nickname },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /**
   * Clears a member's nickname in a community: anyone's own, or, with Manage nicknames, that of
   * someone ranked below the caller. The change arrives as the membership's update event.
   */
  async clearNickname(communityId: string, userId: string): Promise<void> {
    const result = await this.#client.api.DELETE(
      "/api/v1/communities/{community}/members/{user}/nickname",
      { params: { path: { community: communityId, user: userId } } },
    );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
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
   * Saves a message for the caller, or stops saving it. The store follows at once; the
   * `savedMessageChanged` event that follows changes nothing more.
   */
  async setSaved(messageId: string, saved: boolean): Promise<void> {
    const params = { path: { message: messageId } };
    if (saved) {
      const result = await this.#client.api.PUT("/api/v1/users/@me/saved-messages/{message}", {
        params,
      });
      if (result.data === undefined) {
        throw new ApiProblemError(problemOf(result.error, result.response));
      }
      this.store.applyEvent({
        serverEvent: "savedMessageChanged",
        message: messageId,
        saved: result.data.id,
      });
    } else {
      const result = await this.#client.api.DELETE("/api/v1/users/@me/saved-messages/{message}", {
        params,
      });
      if (result.error !== undefined) {
        throw new ApiProblemError(problemOf(result.error, result.response));
      }
      this.store.applyEvent({
        serverEvent: "savedMessageChanged",
        message: messageId,
        saved: null,
      });
    }
  }

  /**
   * The messages the caller saved, newest save first, a page of `SAVED_PAGE` after the save of
   * `before`. Their authors, channels, attachments, polls, and reactions are cached.
   */
  async loadSavedMessages(before?: string): Promise<Message[]> {
    const result = await this.#client.api.GET("/api/v1/users/@me/saved-messages/messages", {
      params: {
        query: {
          ...(before === undefined ? {} : { before }),
          limit: SAVED_PAGE,
          include: [...LISTED_INCLUDES],
        },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.#ingestListed(result.data);
    return result.data.data;
  }

  /** Follows a thread for the caller, or stops following it. The store follows at once. */
  async setFollowing(threadId: string, following: boolean): Promise<void> {
    const params = { path: { channel: threadId } };
    const result = following
      ? await this.#client.api.PUT("/api/v1/channels/{channel}/follows/@me", { params })
      : await this.#client.api.DELETE("/api/v1/channels/{channel}/follows/@me", { params });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.store.applyEvent({ serverEvent: "threadFollowChanged", thread: threadId, following });
  }

  /**
   * A page of `ACTIVITY_PAGE` of the caller's activity feed on this deployment, newest first:
   * the messages that tell them of themselves. Their authors, channels (threads among them),
   * read positions, attachments, polls, and reactions are cached.
   */
  async readActivity(filter: ActivityFilter): Promise<Message[]> {
    const result = await this.#client.api.GET("/api/v1/users/@me/activity", {
      params: {
        query: {
          ...(filter.communities === undefined
            ? {}
            : { "filter[community]": [...filter.communities] }),
          "filter[dms]": filter.dms,
          "filter[unread]": filter.unread,
          ...(filter.before === undefined ? {} : { before: filter.before }),
          limit: ACTIVITY_PAGE,
          include: [...LISTED_INCLUDES],
        },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.#ingestListed(result.data);
    return result.data.data;
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
   * Reads the commands of the bots that can see a channel into the store, once however many
   * ask at the same time; the store drops them when something that may change them happens.
   */
  loadCommands(channelId: string): Promise<void> {
    const pending = this.#commandLoads.get(channelId);
    if (pending !== undefined) {
      return pending;
    }
    const load = (async () => {
      const result = await this.#client.api.GET("/api/v1/channels/{channel}/commands", {
        params: { path: { channel: channelId } },
      });
      if (result.data === undefined) {
        throw new ApiProblemError(problemOf(result.error, result.response));
      }
      this.store.setCommands(channelId, result.data);
    })().finally(() => {
      this.#commandLoads.delete(channelId);
    });
    this.#commandLoads.set(channelId, load);
    return load;
  }

  /**
   * Reads the emoji the user reacts with most in `communityId` (`null`: a DM) into the store,
   * once however many ask at the same time; the store marks them to be read again when one of
   * the user's own reactions comes or goes.
   */
  loadFrequentEmoji(communityId: string | null): Promise<void> {
    const key = communityId ?? "";
    const pending = this.#frequentEmojiLoads.get(key);
    if (pending !== undefined) {
      return pending;
    }
    const load = (async () => {
      const asOf = this.store.reactionChanges;
      const result = await this.#client.api.GET("/api/v1/users/{user}/frequent-emoji", {
        params: {
          path: { user: "@me" },
          query: communityId === null ? {} : { community: communityId },
        },
      });
      if (result.data === undefined) {
        throw new ApiProblemError(problemOf(result.error, result.response));
      }
      this.store.setFrequentEmoji(
        communityId,
        result.data.map((f) => f.emoji),
        asOf,
      );
    })().finally(() => {
      this.#frequentEmojiLoads.delete(key);
    });
    this.#frequentEmojiLoads.set(key, load);
    return load;
  }

  /**
   * Sends a bot a command in a channel. The server checks its arguments and posts it there as
   * a message of kind `command`, which arrives like any other.
   */
  async invokeCommand(channelId: string, invocation: Invocation): Promise<void> {
    const result = await this.#client.api.POST("/api/v1/channels/{channel}/commands", {
      params: { path: { channel: channelId } },
      body: invocation,
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
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
          include: ["authors", "memberships", "attachments", "polls", "channels", "reactions"],
        },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.#ingestMessageRead(result.data.included, result.data.data);
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
      // A new username composed (NFC), as the server requires.
      body: patch.name == null ? patch : { ...patch, name: patch.name.normalize("NFC") },
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

  /** Loads the first page of a community's invites into the store, or, with `more`, the next. */
  async loadInvites(communityId: string, more = false): Promise<void> {
    const before = more ? this.store.invites(communityId).at(-1)?.code : undefined;
    if (more && (before === undefined || this.store.listComplete(`invites:${communityId}`))) {
      return;
    }
    const result = await this.#client.api.GET("/api/v1/communities/{community}/invites", {
      params: {
        path: { community: communityId },
        query: before === undefined ? { limit: LIST_PAGE } : { before, limit: LIST_PAGE },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    const complete = result.data.length < LIST_PAGE;
    if (more) {
      this.store.appendInvites(communityId, result.data, complete);
    } else {
      this.store.replaceInvites(communityId, result.data, complete);
    }
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
            "emoji",
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
      this.store.missing("user", userId) ||
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
          this.store.markMissing("user", userId);
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

  /** The people of `ids`, each as cached or read now; `undefined` for one who is not found. */
  async loadUsers(ids: readonly string[]): Promise<(User | undefined)[]> {
    for (const id of ids) {
      this.ensureUser(id);
    }
    await Promise.all(ids.flatMap((id) => this.#userLoads.get(id) ?? []));
    return ids.map((id) => this.store.user(id));
  }

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
      const [me, communities, dms, admin, blocks, plugins, heldMessages, saves, follows] =
        await Promise.all([
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
                  "emoji",
                ],
              },
            },
          }),
          this.#client.api.GET("/api/v1/users/@me/dms", {
            params: {
              query: {
                include: ["users", "readStates", "mutes", "notifications", "voice"],
                limit: DM_PAGE,
              },
            },
          }),
          this.#client.api.GET("/api/v1/users/@me/admin"),
          this.#client.api.GET("/api/v1/users/@me/blocks", {
            params: { query: { include: ["users"] } },
          }),
          this.#client.api.GET("/api/v1/plugins"),
          this.#readHeldMessages(),
          this.#client.api.GET("/api/v1/users/@me/saved-messages"),
          this.#client.api.GET("/api/v1/users/@me/thread-follows"),
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
      this.store.setDms(dms.data.data, dms.data.data.length < DM_PAGE);
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
      // A deployment without plugins, or one that does not say, runs none.
      this.store.setPlugins(plugins.data ?? []);
      // Read before the events held back meanwhile, applied below, which settle any posted
      // since; a deployment that does not hold messages has none.
      this.store.replaceHeldMessages(heldMessages);
      // A deployment that keeps no saves or follows has none.
      this.store.replaceSaves(saves.data ?? []);
      this.store.replaceFollows((follows.data ?? []).map((follow) => follow.thread));
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
   * Keeps a channel's online count current while it is shown: read at once when the sync is
   * live, then with every presence poll. Returns what stops it.
   */
  watchChannelPresence(channelId: string): () => void {
    const watchers = this.#presenceChannels.get(channelId) ?? 0;
    this.#presenceChannels.set(channelId, watchers + 1);
    if (watchers === 0 && this.#isLive()) {
      void this.#loadChannelPresence(channelId).catch(() => undefined);
    }
    return () => {
      const left = (this.#presenceChannels.get(channelId) ?? 1) - 1;
      if (left > 0) {
        this.#presenceChannels.set(channelId, left);
      } else {
        this.#presenceChannels.delete(channelId);
      }
    };
  }

  async #loadChannelPresence(channelId: string): Promise<void> {
    const result = await this.#client.api.GET("/api/v1/channels/{channel}/presence", {
      params: { path: { channel: channelId } },
    });
    if (result.data !== undefined) {
      this.store.setChannelOnline(channelId, result.data.online);
    }
  }

  /**
   * Asks the server for the presence of everyone on screen and the online count of each channel
   * shown, then again after
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
      await Promise.all([
        ...batches.map(async (batch) => {
          const result = await this.#client.api.GET("/api/v1/users/statuses", {
            params: { query: { ids: batch.join(",") } },
          });
          if (result.data !== undefined) {
            this.store.applyStatuses(result.data);
          }
        }),
        ...Array.from(this.#presenceChannels.keys(), (id) => this.#loadChannelPresence(id)),
      ]).catch(() => undefined);
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

  /**
   * Shows who is typing in a channel while the returned function is not yet called: the server
   * sends typing only for channels the app says it shows (`viewing`), at most `MAX_VIEWING`.
   */
  watchTyping(channelId: string): () => void {
    this.#viewing.set(channelId, (this.#viewing.get(channelId) ?? 0) + 1);
    this.#tellViewing();
    return () => {
      const held = (this.#viewing.get(channelId) ?? 1) - 1;
      if (held > 0) {
        this.#viewing.set(channelId, held);
      } else {
        this.#viewing.delete(channelId);
      }
      this.#tellViewing();
    };
  }

  /** Tells the server which channels are shown, the latest first when there are too many. */
  #tellViewing(): void {
    this.#stream.sendViewing(Array.from(this.#viewing.keys()).reverse().slice(0, MAX_VIEWING));
  }

  /**
   * The user wrote in a channel's message box. The server hears that they are typing there at
   * most every `TYPING_REFRESH_MS`, and not at all while the user has turned typing notices off
   * (`TYPING_NOTICES`).
   */
  noteTyping(channelId: string): void {
    if (!this.preferences.get(TYPING_NOTICES)) {
      return;
    }
    const now = this.#now();
    const sent = this.#typingSent.get(channelId);
    if (sent !== undefined && now - sent < TYPING_REFRESH_MS) {
      return;
    }
    if (this.#stream.sendTyping(channelId, true)) {
      this.#typingSent.set(channelId, now);
    }
  }

  /**
   * The user stopped typing in a channel: they sent the message, emptied the box, or left it.
   * Said only where the server was told they were typing.
   */
  stopTyping(channelId: string): void {
    if (this.#typingSent.delete(channelId)) {
      this.#stream.sendTyping(channelId, false);
    }
  }

  /** Lets go of those whose typing ran out, and waits for the next to. */
  #expireTyping(): void {
    if (this.#typingTimer !== null) {
      clearTimeout(this.#typingTimer);
      this.#typingTimer = null;
    }
    const now = this.#now();
    const next = this.store.expireTyping(now);
    if (next !== null) {
      this.#typingTimer = this.#setTimeout(
        () => {
          this.#typingTimer = null;
          this.#expireTyping();
        },
        Math.min(next - now, MAX_TIMER_MS),
      );
    }
  }

  #forgetTyping(): void {
    if (this.#typingTimer !== null) {
      clearTimeout(this.#typingTimer);
      this.#typingTimer = null;
    }
    this.#typingSent.clear();
    this.store.forgetTyping();
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
    this.#tellViewing();
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
    // Something the user's own subject told of (their DMs, memberships, settings) may not have
    // happened; everything held is read again.
    if (event.serverEvent === "userResync") {
      void this.#resync();
      return;
    }
    if (event.serverEvent === "pluginEvent") {
      for (const listener of this.#pluginEventListeners) {
        listener(event);
      }
      return;
    }
    if (event.serverEvent === "pluginNotice") {
      for (const listener of this.#pluginNoticeListeners) {
        listener(event);
      }
      return;
    }
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
    const resamples = this.#mayResampleMembers(event);
    const unheld = this.#unheldChannelIn(event);
    const moderates =
      event.serverEvent === "deploymentAccessChanged" &&
      !this.store.moderator &&
      event.permissions.includes("moderateCommunities");
    const retagged = this.#unreadTagsChangedBy(event);
    const blockChanged =
      event.serverEvent === "userBlockChanged" && this.store.blocked(event.user) !== event.blocked;
    if (this.#namesUnknownPlugin(event)) {
      void this.loadPlugins().catch(() => undefined);
    }
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
    if (resamples !== null) {
      this.#scheduleMemberResample(resamples);
    }
    if (unheld !== null) {
      this.#scheduleDiscovery(unheld);
    }
    // Moderating the deployment shows every channel of every community, which the caller's
    // own reads left out.
    if (moderates) {
      for (const community of this.store.communities()) {
        this.#scheduleAccessReload(community.id);
      }
    }
    // Something announced about the community may not have happened; it is read again.
    if (event.serverEvent === "communityResync") {
      this.#scheduleAccessReload(event.community);
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
    if (event.serverEvent === "voiceMute" && event.user === this.store.me()?.id) {
      // A moderator's mute of the user, in the community of the call they are in.
      const callChannel = this.voice.state.channelId;
      const community =
        callChannel === null ? undefined : this.store.channel(callChannel)?.community;
      if (community != null && community === event.community) {
        this.voice.setServerMuted(event.type === "create");
      }
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
        // Deleting a role the caller holds may lift a denial it carried in channels whose
        // overrides the caller was never sent.
        if (event.type !== "delete" && (event.type !== "update" || event.permissions == null)) {
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
        // One about a category the caller does not hold reaches them because they may now
        // learn of it (the server sends a category's events to those its own overrides let
        // view it, before or after the change).
        const category = this.store.category(event.category);
        const community = category?.community ?? this.#communityOfRole(event.role);
        return community !== undefined && (category === undefined || holds(community, event.role))
          ? community
          : null;
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

  /**
   * A community channel `event` concerns that the store does not hold: an override set on it,
   * or its move. The server sends these to whoever may view the channel before or after the
   * change, so one about a channel the caller does not have is one they may view now.
   */
  #unheldChannelIn(event: ServerEvent): string | null {
    const channel =
      event.serverEvent === "channelOverride"
        ? event.channel
        : event.serverEvent === "channel" &&
            event.type === "update" &&
            event.parentCategory !== undefined
          ? event.id
          : null;
    return channel !== null && this.store.channel(channel) === undefined ? channel : null;
  }

  /**
   * Looks up a channel the caller may now view, and then reads its community again, which
   * brings the channel with everything about it. It waits a moment, spread like an access
   * reload, since a new channel's overrides arrive just ahead of the channel itself.
   */
  #scheduleDiscovery(channelId: string): void {
    if (this.#discoveries.has(channelId)) {
      return;
    }
    this.#discoveries.add(channelId);
    const generation = this.#generation;
    this.#setTimeout(() => {
      this.#discoveries.delete(channelId);
      if (generation !== this.#generation || this.store.channel(channelId) !== undefined) {
        return;
      }
      void this.#client.api
        .GET("/api/v1/channels/{channel}", { params: { path: { channel: channelId } } })
        .then((result) => {
          const community = result.data?.community;
          if (community != null && generation === this.#generation) {
            this.#scheduleAccessReload(community);
          }
        })
        .catch(() => undefined);
    }, this.#random() * ACCESS_RELOAD_SPREAD_MS);
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
   * The community whose member sample `event` may change, before it is applied: the server puts
   * connected holders of roles shown apart first, by their rank, so a role coming to be shown
   * apart or not, one shown apart moving or going, or one given or taken, may change who is in
   * it.
   */
  #mayResampleMembers(event: ServerEvent): string | null {
    switch (event.serverEvent) {
      case "role": {
        const community = this.#communityOfRole(event.id);
        if (community === undefined) {
          return null;
        }
        const shownApart =
          this.store.roles(community).find((r) => r.id === event.id)?.hoist === true;
        const changes =
          event.type === "delete"
            ? shownApart
            : event.type === "update" &&
              ((event.hoist != null && event.hoist !== shownApart) ||
                (shownApart && event.position != null));
        return changes ? community : null;
      }
      case "userCommunity": {
        if (event.type !== "update" || event.roles == null) {
          return null;
        }
        const before = new Set(this.store.memberRoles(event.community, event.user) ?? []);
        const after = new Set(event.roles);
        const shownApart = this.store
          .roles(event.community)
          .some((r) => r.hoist && before.has(r.id) !== after.has(r.id));
        return shownApart ? event.community : null;
      }
      default:
        return null;
    }
  }

  /**
   * Reads a community's member sample again soon, at a random moment within
   * `MEMBER_RESAMPLE_SPREAD_MS`, once however many changes ask meanwhile.
   */
  #scheduleMemberResample(communityId: string): void {
    if (this.#memberResamples.has(communityId)) {
      return;
    }
    this.#memberResamples.add(communityId);
    const generation = this.#generation;
    this.#setTimeout(() => {
      this.#memberResamples.delete(communityId);
      if (generation === this.#generation) {
        void this.loadMemberSample(communityId).catch(() => undefined);
      }
    }, this.#random() * MEMBER_RESAMPLE_SPREAD_MS);
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

  /** Caches a list of messages read from outside any one channel, and what came with them. */
  #ingestListed(read: { data: NonNullable<Included["messages"]>; included: Included }): void {
    this.#ingestMessageRead(read.included, read.data);
    this.store.setReactions(
      read.data.map((m) => m.id),
      read.included.reactions ?? [],
    );
  }

  /**
   * Takes in what a message read sideloaded, with `messages` when the read's own are to be
   * stored too. Its memberships say which roles the authors hold and what they are called
   * there, for drawing their names in their roles' colours and by their nicknames; they are not
   * the community's member sample, which they leave alone.
   */
  #ingestMessageRead(included: Included, messages?: Included["messages"]): void {
    const { userCommunities, ...rest } = included;
    this.store.ingest(messages === undefined ? rest : { ...rest, messages: [...messages] });
    this.store.noteMemberships(userCommunities ?? []);
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
          include: [
            "authors",
            "memberships",
            "attachments",
            "polls",
            "threads",
            "echoes",
            "reactions",
            "linked",
            "warnings",
            "annotations",
          ],
        },
      },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.#ingestMessageRead(result.data.included);
    this.store.setReactions(
      result.data.data.map((m) => m.id),
      result.data.included.reactions ?? [],
    );
    // Annotations come for the messages read and for those the read names (echoes' replies).
    this.store.setAnnotations(
      [
        ...result.data.data.map((m) => m.id),
        ...(result.data.included.messages ?? []).map((m) => m.id),
      ],
      result.data.included.messageAnnotations ?? [],
    );
    return result.data.data;
  }
}
