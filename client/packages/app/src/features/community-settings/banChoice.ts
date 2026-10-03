/** What a ban form chooses, and the request it makes, shared by every ban form (`BanFields`). */

/** How long a ban may be chosen to last, in seconds; `null` until lifted. */
export const DURATIONS = [
  { id: "forever", seconds: null },
  { id: "hour", seconds: 3_600 },
  { id: "day", seconds: 86_400 },
  { id: "week", seconds: 7 * 86_400 },
  { id: "month", seconds: 30 * 86_400 },
] as const;

/** How far back a ban may delete the person's messages, in seconds; `null` keeps them. */
export const DELETE_WINDOWS = [
  { id: "none", seconds: null },
  { id: "hour", seconds: 3_600 },
  { id: "day", seconds: 86_400 },
] as const;

type DurationId = (typeof DURATIONS)[number]["id"];
type WindowId = (typeof DELETE_WINDOWS)[number]["id"];

/** What a ban form holds: a reason, how long, and how far back messages go with it. */
export interface BanChoice {
  reason: string;
  duration: DurationId;
  window: WindowId;
}

export const NEW_BAN: BanChoice = { reason: "", duration: "forever", window: "none" };

/** The request a ban form makes, leaving out the deletion where the banner may not delete. */
export function banRequest(
  choice: BanChoice,
  mayDelete: boolean,
): { reason?: string; durationSeconds?: number; deleteMessagesSeconds?: number } {
  const seconds = DURATIONS.find((d) => d.id === choice.duration)?.seconds ?? null;
  const deleting = mayDelete
    ? (DELETE_WINDOWS.find((w) => w.id === choice.window)?.seconds ?? null)
    : null;
  return {
    ...(choice.reason.trim() === "" ? {} : { reason: choice.reason.trim() }),
    ...(seconds === null ? {} : { durationSeconds: seconds }),
    ...(deleting === null ? {} : { deleteMessagesSeconds: deleting }),
  };
}
