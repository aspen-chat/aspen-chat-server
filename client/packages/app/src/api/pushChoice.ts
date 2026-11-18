import { useSyncExternalStore } from "react";

/**
 * Whether this install's person chose to be woken by push: `on` once they accepted the prompt or
 * turned it on in Settings, `off` once they declined it or turned it off there, `null` while they
 * have not answered. Kept per install, since it is this device's to decide, not the account's.
 */
export type PushChoice = "on" | "off" | null;

/** What the mobile app can say of push on this device, for the prompt and Settings to show. */
export interface PushStatus {
  readonly choice: PushChoice;
  /** Whether this build can be woken at all: it names a relay its publisher runs. */
  readonly available: boolean;
  /** Whether the system refuses Aspen's notifications, which only its own settings change. */
  readonly denied: boolean;
  /** Whether the prompt is waiting for an answer. */
  readonly asking: boolean;
  /** How many answers this start has had, so one repeating the last is followed again. */
  readonly answers: number;
}

const STORAGE_KEY = "aspen.push";

function storedChoice(): PushChoice {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored === "on" || stored === "off" ? stored : null;
  } catch {
    return null;
  }
}

let status: PushStatus = {
  choice: storedChoice(),
  available: false,
  denied: false,
  asking: false,
  answers: 0,
};
const listeners = new Set<() => void>();

function update(next: Partial<PushStatus>): void {
  const updated = { ...status, ...next };
  if (
    updated.choice === status.choice &&
    updated.available === status.available &&
    updated.denied === status.denied &&
    updated.asking === status.asking &&
    updated.answers === status.answers
  ) {
    return;
  }
  status = updated;
  for (const listener of listeners) {
    listener();
  }
}

export function pushStatus(): PushStatus {
  return status;
}

export function subscribePush(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePushStatus(): PushStatus {
  return useSyncExternalStore(subscribePush, pushStatus);
}

/** Records the person's answer, which `WakeThisPhone` follows, and puts the prompt away. */
export function choosePush(choice: "on" | "off"): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, choice);
  } catch {
    // Storage refused: the answer holds until the app restarts, which then asks again.
  }
  update({ choice, asking: false, answers: status.answers + 1 });
}

/** Puts the prompt away unanswered; the next start asks again. */
export function dismissPushPrompt(): void {
  update({ asking: false });
}

export function notePushStatus(next: Partial<Omit<PushStatus, "choice" | "answers">>): void {
  update(next);
}
