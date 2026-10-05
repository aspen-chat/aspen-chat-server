import { ANNOUNCE_MESSAGES, type AspenSync } from "@aspen/protocol";
import { useEffect, useRef } from "react";
import { usePreference } from "@/api/hooks";
import { announce } from "@/features/layout/announce";
import { describe } from "@/features/notifications/describe";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** How recently a message must have arrived to be read out, so a list opened later is quiet. */
const FRESH_MS = 10_000;

/**
 * Reads out each message that arrives in a list on screen, where the reader asked for it
 * (`ANNOUNCE_MESSAGES`): who wrote it and what it says. The reader's own messages, those of
 * people they blocked, and history read in are left unsaid.
 */
export function useAnnounceArrivals(sync: AspenSync, ids: readonly string[] | undefined): void {
  const m = useMessages();
  const wanted = usePreference(ANNOUNCE_MESSAGES);
  const said = useRef(new Set<string>());
  useEffect(() => {
    if (!wanted || ids === undefined) {
      return;
    }
    const store = sync.store;
    const me = store.me()?.id;
    const now = Date.now();
    for (const id of ids) {
      const arrived = store.arrivedAt(id);
      const message = store.message(id);
      if (
        arrived === undefined ||
        now - arrived > FRESH_MS ||
        message === undefined ||
        said.current.has(id)
      ) {
        continue;
      }
      said.current.add(id);
      if (message.author === me || store.blocked(message.author)) {
        continue;
      }
      const { name, body } = describe(m, sync, message);
      announce(format(m.notifications.announced, { name, body }));
    }
  }, [wanted, ids, sync, m]);
}
