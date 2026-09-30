import type { Attachment } from "@aspen/protocol";
import type { PickedTag } from "@/features/mentions/tags";

/**
 * Drafts: what someone has written in a message box and not sent, kept on this device per
 * account and per channel (a thread is a channel of its own), so it waits for them wherever
 * they went meanwhile: another channel, a notification, a reload. Files already uploaded stay
 * with it; the server keeps an uploaded file until it is sent or removed.
 *
 * A box notes its draft here as it changes (`noteDraft`), in memory, and keeps it in storage a
 * moment later (`writeDraft`). A box that takes another's place, as when a notification opens
 * another channel, renders before the one leaving has kept its draft, so what is read comes from
 * memory first. In storage all drafts are one entry of `localStorage`, read and written whole:
 * they are few and small, and the oldest go first past `MAX_DRAFTS` or `MAX_AGE_MS`. Storage that
 * is missing, full, or refused leaves drafts for the page's life only.
 */

export interface Draft {
  readonly text: string;
  /** The tags picked in it, so they are sent as tags (`encodeTags`). */
  readonly picks: readonly PickedTag[];
  /** Files uploaded for it. */
  readonly attachments: readonly Attachment[];
  /** In a thread, whether to show the reply in its parent channel too. */
  readonly echo: boolean;
  readonly savedAt: number;
}

const STORAGE_KEY = "aspen.drafts";
export const MAX_DRAFTS = 200;
export const MAX_AGE_MS = 30 * 24 * 60 * 60 * 1000;

type Drafts = Record<string, Draft>;

/** Drafts of this page's life, for when storage cannot hold them. */
let fallback: Drafts = {};

/** The latest draft of each box this page has had, kept or not; `null` for none. */
const noted = new Map<string, Draft | null>();

function keyOf(userId: string, channelId: string): string {
  return `${userId}:${channelId}`;
}

function readAll(): Drafts {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored === null ? {} : (JSON.parse(stored) as Drafts);
  } catch {
    return fallback;
  }
}

function writeAll(drafts: Drafts): void {
  fallback = drafts;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(drafts));
  } catch {
    // Kept in `fallback` for this page.
  }
}

/** Whether a draft holds nothing worth keeping. */
export function isEmpty(draft: Pick<Draft, "text" | "attachments">): boolean {
  return draft.text.trim() === "" && draft.attachments.length === 0;
}

/** The draft of `channelId` for `userId`, if one was noted or kept. */
export function readDraft(userId: string, channelId: string): Draft | null {
  const key = keyOf(userId, channelId);
  const latest = noted.get(key);
  return latest !== undefined ? latest : (readAll()[key] ?? null);
}

/** Notes the draft of `channelId` as it stands, for `readDraft`, before it is kept. */
export function noteDraft(
  userId: string,
  channelId: string,
  draft: Omit<Draft, "savedAt"> | null,
  now = Date.now(),
): void {
  noted.set(
    keyOf(userId, channelId),
    draft === null || isEmpty(draft) ? null : { ...draft, savedAt: now },
  );
}

/**
 * Keeps `draft` for `channelId`, or forgets it when it is empty or `null`, and lets the oldest
 * drafts go when there are too many or they are too old.
 */
export function writeDraft(
  userId: string,
  channelId: string,
  draft: Omit<Draft, "savedAt"> | null,
  now = Date.now(),
): void {
  noteDraft(userId, channelId, draft, now);
  const key = keyOf(userId, channelId);
  const forget = draft === null || isEmpty(draft);
  const all = readAll();
  if (forget && !(key in all)) {
    return;
  }
  const others = Object.entries(all).filter(([k]) => k !== key);
  const entries = forget ? others : [...others, [key, { ...draft, savedAt: now }] as const];
  const kept = entries
    .filter(([, d]) => now - d.savedAt <= MAX_AGE_MS)
    .sort(([, a], [, b]) => b.savedAt - a.savedAt)
    .slice(0, MAX_DRAFTS);
  const keptKeys = new Set(kept.map(([k]) => k));
  for (const [k] of entries) {
    if (!keptKeys.has(k)) {
      noted.delete(k);
    }
  }
  writeAll(Object.fromEntries(kept));
}
