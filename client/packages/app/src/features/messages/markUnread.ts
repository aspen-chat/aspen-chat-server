import { ApiProblemError, type AspenSync } from "@aspen/protocol";
import { toast } from "@/features/layout/toast";
import type { Messages } from "@/i18n/messages";

/**
 * Marks a channel, DM, or thread unread from `messageId` on, or from its newest message by
 * someone else when none is named (`AspenSync.markUnread`), and says how it went in a toast:
 * the list it was marked in shows nothing else of it while it is open.
 */
export function markUnread(
  sync: AspenSync,
  m: Messages,
  channelId: string,
  messageId?: string,
): void {
  sync.markUnread(channelId, messageId).then(
    () => {
      toast(m.markedUnread);
    },
    (e: unknown) => {
      toast(e instanceof ApiProblemError ? e.message : String(e));
    },
  );
}
