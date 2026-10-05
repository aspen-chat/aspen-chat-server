import { useSyncExternalStore } from "react";

/**
 * Text for assistive technology to read out politely, from anywhere: `announce` adds it to one
 * live region, which `Announcer` keeps at the root of the page. Each announcement is a node of
 * its own, the last few kept, since screen readers read what is added to a region more reliably
 * than what changes in it.
 */

/** How many announcements the region holds; older ones are long since read. */
const KEPT = 5;

export interface Announcement {
  readonly id: number;
  readonly text: string;
}

let announcements: readonly Announcement[] = [];
let nextId = 0;
const listeners = new Set<() => void>();

export function announce(text: string): void {
  announcements = [...announcements, { id: nextId++, text }].slice(-KEPT);
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** What the live region holds now. */
export function useAnnouncements(): readonly Announcement[] {
  return useSyncExternalStore(subscribe, () => announcements);
}
