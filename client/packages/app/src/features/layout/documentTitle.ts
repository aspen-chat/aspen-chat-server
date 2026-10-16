import { useEffect, useSyncExternalStore } from "react";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The page's title, which browsers show on its tab, the system on the window and its taskbar
 * button, and the desktop app in the title bar it draws (`TitleBar.tsx`). It names where the
 * reader is no more closely than a community, never a channel or a conversation.
 *
 * Each screen names itself with `useDocumentTitle`; the one mounted last is the one shown, and the
 * app's name alone stands while none is mounted or the one shown is still loading.
 */

interface Naming {
  readonly page: string | null;
}

let namings: readonly Naming[] = [];
const listeners = new Set<() => void>();

function notify(): void {
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

function shownPage(): string | null {
  return namings.at(-1)?.page ?? null;
}

/** Names the page after `page` while the calling screen is mounted; `null` while it is loading. */
export function useDocumentTitle(page: string | null): void {
  useEffect(() => {
    const naming: Naming = { page };
    namings = [...namings, naming];
    notify();
    return () => {
      namings = namings.filter((other) => other !== naming);
      notify();
    };
  }, [page]);
}

/** The page's title, in the reader's language. */
export function useTitle(): string {
  const m = useMessages();
  const page = useSyncExternalStore(subscribe, shownPage);
  return page === null ? m.appName : format(m.documentTitle, { page, app: m.appName });
}

/** Keeps `document.title` the page's title. Mounted once, at the root. */
export function FollowDocumentTitle(): null {
  const title = useTitle();
  useEffect(() => {
    document.title = title;
  }, [title]);
  return null;
}
