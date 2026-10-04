import { type AspenClient, problemOf } from "./http";
import { ApiProblemError } from "./problem";

/**
 * User preferences. Each preference declares its scope: a `device` preference belongs to this
 * install (this browser profile, this desktop or mobile app) and lives in the storage the
 * store was given, which is `localStorage` in a page; an `account` preference belongs to the
 * user and lives on the server, where every one of their devices sees it and a change on one
 * reaches the others through the event stream. The store is the one place both are read and
 * written, so the UI does not care which is which.
 *
 * Values are JSON. A definition carries a `parse` that turns whatever was stored into the
 * value's type or `undefined`, so a stale or malformed entry falls back to the default
 * instead of surprising the code that reads it.
 */

export type PreferenceScope = "device" | "account";

export interface PreferenceDefinition<T> {
  /** Stable, namespaced, such as `audio.input`. It is the storage key and the server key. */
  readonly key: string;
  readonly scope: PreferenceScope;
  readonly fallback: T;
  readonly parse: (raw: unknown) => T | undefined;
}

/** One preference and a value for it, for `PreferenceStore.setAccount`. */
export interface PreferenceValue {
  readonly definition: PreferenceDefinition<unknown>;
  readonly value: unknown;
}

/** Pairs a preference with a value of its own type. */
export function preferenceValue<T>(definition: PreferenceDefinition<T>, value: T): PreferenceValue {
  return { definition: definition, value };
}

/** The subset of `Storage` the store uses, so tests and shells can supply their own. */
export interface PreferenceStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface PreferenceStoreOptions {
  /** Where device preferences live; `null` keeps them in memory for the session only. */
  storage?: PreferenceStorage | null;
  /** The account preferences' server; absent, account preferences stay at their defaults. */
  client?: AspenClient;
}

const STORAGE_PREFIX = "aspen.preference.";

/** Whatever the system routes to. */
export const DEFAULT_DEVICE = "default";
/** For the notification output: the same device voice chat plays through. */
export const SAME_AS_VOICE = "voice";

/**
 * A chosen microphone or speaker. Both the browser's id and the label are kept: ids are
 * salted per site and can change across sessions and after permission changes, so when the
 * id no longer matches a device the label finds it again.
 */
export interface NamedDevice {
  readonly id: string;
  readonly label: string;
}

export type DeviceChoice = typeof DEFAULT_DEVICE | NamedDevice;
export type NotificationChoice = typeof SAME_AS_VOICE | DeviceChoice;

function parseDevice(raw: unknown): DeviceChoice | undefined {
  if (raw === DEFAULT_DEVICE) {
    return raw;
  }
  if (typeof raw === "object" && raw !== null && "id" in raw && "label" in raw) {
    const { id, label } = raw;
    if (typeof id === "string" && id.length > 0 && id.length <= 512 && typeof label === "string") {
      return { id, label: label.slice(0, 256) };
    }
  }
  return undefined;
}

function parseNotification(raw: unknown): NotificationChoice | undefined {
  return raw === SAME_AS_VOICE ? raw : parseDevice(raw);
}

/** The microphone voice chat captures. */
export const AUDIO_INPUT: PreferenceDefinition<DeviceChoice> = {
  key: "audio.input",
  scope: "device",
  fallback: DEFAULT_DEVICE,
  parse: parseDevice,
};

/** The camera calls send, when the user turns it on. */
export const VIDEO_INPUT: PreferenceDefinition<DeviceChoice> = {
  key: "video.input",
  scope: "device",
  fallback: DEFAULT_DEVICE,
  parse: parseDevice,
};

/** The speaker voice chat plays through. */
export const AUDIO_OUTPUT: PreferenceDefinition<DeviceChoice> = {
  key: "audio.output",
  scope: "device",
  fallback: DEFAULT_DEVICE,
  parse: parseDevice,
};

/** The speaker notification sounds play through; `SAME_AS_VOICE` follows `AUDIO_OUTPUT`. */
export const NOTIFICATION_OUTPUT: PreferenceDefinition<NotificationChoice> = {
  key: "audio.notificationOutput",
  scope: "device",
  fallback: SAME_AS_VOICE,
  parse: parseNotification,
};

/**
 * Whether this install shows the system's notifications for messages the user's notification
 * settings ask to be told of, while the app is open but not looking at them. Off until the user
 * turns it on, which is when the browser asks their permission.
 */
export const DESKTOP_NOTIFICATIONS: PreferenceDefinition<boolean> = {
  key: "notifications.desktop",
  scope: "device",
  fallback: false,
  parse: (raw) => (typeof raw === "boolean" ? raw : undefined),
};

/** Whether this install plays a sound for such messages. */
export const NOTIFICATION_SOUNDS: PreferenceDefinition<boolean> = {
  key: "notifications.sounds",
  scope: "device",
  fallback: true,
  parse: (raw) => (typeof raw === "boolean" ? raw : undefined),
};

/**
 * Whether this install draws people's names in their roles' colours. On unless the user turns it
 * off, for whom coloured text is harder to read.
 */
export const NAME_COLORS: PreferenceDefinition<boolean> = {
  key: "look.nameColors",
  scope: "device",
  fallback: true,
  parse: (raw) => (typeof raw === "boolean" ? raw : undefined),
};

/** How loud one other person is to this user: a gain, 1 being as sent, up to double. */
export const MAX_USER_VOLUME = 2;

export function userVolume(userId: string): PreferenceDefinition<number> {
  return {
    key: `voice.volume.${userId}`,
    scope: "device",
    fallback: 1,
    parse: (raw) =>
      typeof raw === "number" && Number.isFinite(raw) && raw >= 0 && raw <= MAX_USER_VOLUME
        ? raw
        : undefined,
  };
}

/**
 * Whether the user has opted into developer mode, which shows what developers need: making
 * and managing bots. It follows the account, so it holds on every device they use.
 */
export const DEVELOPER_MODE: PreferenceDefinition<boolean> = {
  key: "developer.mode",
  scope: "account",
  fallback: false,
  parse: (raw) => (typeof raw === "boolean" ? raw : undefined),
};

/**
 * The ID wizard, a part of developer mode: while both are on, everything with an id offers to
 * copy it, last in whatever shows it. On with developer mode unless turned off.
 */
export const ID_WIZARD: PreferenceDefinition<boolean> = {
  key: "developer.idWizard",
  scope: "account",
  fallback: true,
  parse: (raw) => (typeof raw === "boolean" ? raw : undefined),
};

/**
 * How the user arranged the community rail, across every deployment they use: each entry's
 * `railKey`. Communities it does not name follow the ones it does. It follows the account, so
 * the rail looks the same on every device.
 */
export const RAIL_ORDER: PreferenceDefinition<readonly string[]> = {
  key: "rail.order",
  scope: "account",
  fallback: [],
  parse: (raw) =>
    Array.isArray(raw) && raw.every((entry) => typeof entry === "string") ? raw : undefined,
};

/** The tints a rail folder may take; `accent` is the palette's own. */
export const FOLDER_COLORS = ["accent", "sky", "violet", "rose", "amber", "slate"] as const;
export type FolderColor = (typeof FOLDER_COLORS)[number];

/**
 * A folder of communities on the rail. `members` are rail keys, as `RAIL_ORDER` names
 * communities, in the folder's order; `RAIL_ORDER` names the folder itself as
 * `folder:{id}` where it stands among the rest.
 */
export interface RailFolder {
  readonly id: string;
  /** What the user called it; empty until they name it. */
  readonly name: string;
  readonly color: FolderColor;
  /** Whether it is unfolded in the rail, which follows the account like the rest. */
  readonly open: boolean;
  readonly members: readonly string[];
}

/**
 * The user's folders of communities on the rail. A folder written by a newer client may carry
 * more than this one knows, which is ignored, and a colour this one does not know reads as the
 * accent; an entry that is not a folder at all is dropped.
 */
export const RAIL_FOLDERS: PreferenceDefinition<readonly RailFolder[]> = {
  key: "rail.folders",
  scope: "account",
  fallback: [],
  parse: (raw) => {
    if (!Array.isArray(raw)) {
      return undefined;
    }
    const folders: RailFolder[] = [];
    for (const entry of raw as unknown[]) {
      if (typeof entry !== "object" || entry === null) {
        continue;
      }
      const { id, name, color, open, members } = entry as Record<string, unknown>;
      if (
        typeof id !== "string" ||
        !Array.isArray(members) ||
        !members.every((member) => typeof member === "string")
      ) {
        continue;
      }
      folders.push({
        id,
        name: typeof name === "string" ? name : "",
        color: FOLDER_COLORS.find((known) => known === color) ?? "accent",
        open: open === true,
        members: members,
      });
    }
    return folders;
  },
};

/**
 * The language the app shows, a BCP 47 tag of a catalogue it has, or `automatic` to follow the
 * platform's languages. It follows the account, so every device shows the same one.
 */
export const LANGUAGE: PreferenceDefinition<string> = {
  key: "language",
  scope: "account",
  fallback: "automatic",
  parse: (raw) => (typeof raw === "string" ? raw : undefined),
};

/** The fastest animations may be made, as a multiple of their normal speed. */
export const MAX_MOTION_SPEED = 2;

/**
 * How fast the app's animations run, as a multiple of their normal speed, at which most take
 * 200 milliseconds or less; 0 turns them off. It follows the account, so every device moves
 * alike, while each device's own "reduce motion" setting still keeps movement out of its fades.
 */
export const MOTION_SPEED: PreferenceDefinition<number> = {
  key: "look.motionSpeed",
  scope: "account",
  fallback: 1,
  parse: (raw) =>
    typeof raw === "number" && Number.isFinite(raw) && raw >= 0 && raw <= MAX_MOTION_SPEED
      ? raw
      : undefined,
};

/** Whether this user has silenced one other person for themself, keeping their volume for later. */
export function userMuted(userId: string): PreferenceDefinition<boolean> {
  return {
    key: `voice.muted.${userId}`,
    scope: "device",
    fallback: false,
    parse: (raw) => (typeof raw === "boolean" ? raw : undefined),
  };
}

/** How loud one other person is heard: their volume, or silence while muted for this user. */
export function effectiveUserVolume(store: PreferenceStore, userId: string): number {
  return store.get(userMuted(userId)) ? 0 : store.get(userVolume(userId));
}

/**
 * Which of the devices at hand a choice means, by id first and by label when the id is no
 * longer among them; `null` for the system default or a device that is gone.
 */
export function resolveDevice(
  choice: DeviceChoice,
  devices: readonly Pick<MediaDeviceInfo, "deviceId" | "label">[],
): string | null {
  if (choice === DEFAULT_DEVICE) {
    return null;
  }
  const byId = devices.find((device) => device.deviceId === choice.id);
  if (byId !== undefined) {
    return byId.deviceId;
  }
  const byLabel = devices.find((device) => device.label !== "" && device.label === choice.label);
  return byLabel?.deviceId ?? null;
}

export class PreferenceStore {
  readonly #storage: PreferenceStorage | null;
  readonly #client: AspenClient | null;
  readonly #device = new Map<string, unknown>();
  #account: Record<string, unknown> = {};
  #accountLoaded = false;
  /**
   * Parsed values by scope and key. `get` returns the same object for the same stored value,
   * as React's external-store hook requires of a snapshot; parsing afresh each time would
   * hand out a new object per call.
   */
  readonly #parsed = new Map<string, unknown>();
  readonly #listeners = new Set<() => void>();

  constructor(options: PreferenceStoreOptions = {}) {
    this.#storage = options.storage ?? null;
    this.#client = options.client ?? null;
  }

  /**
   * Whether the account preferences have been read from the server since sign-in; until they
   * are, an account preference reads as its fallback.
   */
  get accountLoaded(): boolean {
    return this.#accountLoaded;
  }

  get<T>(definition: PreferenceDefinition<T>): T {
    const cacheKey = `${definition.scope}:${definition.key}`;
    if (this.#parsed.has(cacheKey)) {
      return this.#parsed.get(cacheKey) as T;
    }
    const raw =
      definition.scope === "device"
        ? this.#readDevice(definition.key)
        : this.#account[definition.key];
    const value =
      raw === undefined ? definition.fallback : (definition.parse(raw) ?? definition.fallback);
    this.#parsed.set(cacheKey, value);
    return value;
  }

  /** Writes the value; an account preference is on the server once this resolves. */
  async set<T>(definition: PreferenceDefinition<T>, value: T): Promise<void> {
    if (definition.scope === "device") {
      this.#device.set(definition.key, value);
      this.#parsed.delete(`device:${definition.key}`);
      try {
        this.#storage?.setItem(STORAGE_PREFIX + definition.key, JSON.stringify(value));
      } catch {
        // Storage may be unavailable or full; the value still holds for this session.
      }
      this.#notify();
      return;
    }
    await this.setAccount(preferenceValue(definition, value));
  }

  /**
   * Writes several account preferences in one request, so no reader, on this device or
   * another, ever sees some changed without the rest.
   */
  async setAccount(...values: readonly PreferenceValue[]): Promise<void> {
    if (values.some((entry) => entry.definition.scope !== "account")) {
      throw new Error("setAccount writes account preferences only");
    }
    if (this.#client === null) {
      throw new Error("account preferences need a server");
    }
    const result = await this.#client.api.PATCH("/api/v1/users/{user}/preferences", {
      params: { path: { user: "@me" } },
      body: Object.fromEntries(values.map((entry) => [entry.definition.key, entry.value])),
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.#account = result.data.values;
    this.#accountLoaded = true;
    this.#forgetAccountParses();
    this.#notify();
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  /** Reads the account preferences from the server, at sign-in and when an event says they changed. */
  async loadAccount(): Promise<void> {
    if (this.#client === null) {
      return;
    }
    const result = await this.#client.api.GET("/api/v1/users/{user}/preferences", {
      params: { path: { user: "@me" } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    this.#account = result.data.values;
    this.#accountLoaded = true;
    this.#forgetAccountParses();
    this.#notify();
  }

  /** Forgets the account preferences, at sign-out; device preferences stay with the install. */
  clearAccount(): void {
    this.#account = {};
    this.#accountLoaded = false;
    this.#forgetAccountParses();
    this.#notify();
  }

  #forgetAccountParses(): void {
    for (const key of Array.from(this.#parsed.keys())) {
      if (key.startsWith("account:")) {
        this.#parsed.delete(key);
      }
    }
  }

  #readDevice(key: string): unknown {
    if (this.#device.has(key)) {
      return this.#device.get(key);
    }
    let raw: string | null;
    try {
      raw = this.#storage?.getItem(STORAGE_PREFIX + key) ?? null;
    } catch {
      raw = null;
    }
    let value: unknown;
    if (raw !== null) {
      try {
        value = JSON.parse(raw);
      } catch {
        value = undefined;
      }
    }
    this.#device.set(key, value);
    return value;
  }

  #notify(): void {
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

/**
 * How loud one other person's shared stream (the sound of a screen or game they share) is heard
 * on this install, set apart from their voice: a gain, 1 as sent, up to double.
 */
export function streamVolume(userId: string): PreferenceDefinition<number> {
  return {
    key: `voice.streamVolume.${userId}`,
    scope: "device",
    fallback: 1,
    parse: (raw) =>
      typeof raw === "number" && Number.isFinite(raw) && raw >= 0 && raw <= MAX_USER_VOLUME
        ? raw
        : undefined,
  };
}

/** Whether this user has silenced one other person's stream for themself, voice apart. */
export function streamMuted(userId: string): PreferenceDefinition<boolean> {
  return {
    key: `voice.streamMuted.${userId}`,
    scope: "device",
    fallback: false,
    parse: (raw) => (typeof raw === "boolean" ? raw : undefined),
  };
}

/** How loud one other person's stream is heard: its volume, or silence while muted for this user. */
export function effectiveStreamVolume(store: PreferenceStore, userId: string): number {
  return store.get(streamMuted(userId)) ? 0 : store.get(streamVolume(userId));
}
