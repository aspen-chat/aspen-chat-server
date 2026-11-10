import type { AspenSync } from "@aspen/protocol";
import { toast } from "@/features/layout/toast";
import type { Messages } from "@/i18n/messages";
import { format } from "@/i18n/messages";

/**
 * Telling the reader that a message they sent will not be posted because one of its files
 * failed to upload (`AspenSync.uploads`), wherever they are by then: the message says so
 * itself at the foot of its channel (`PendingMessages`), which they may have left. A file that
 * fails in a message box is the box's to say, and one whose page went away is said where it is
 * found.
 */

let words: Messages | null = null;

/** The words of the notice, in the reader's language as it is now. */
export function wordUploadFailures(m: Messages): void {
  words = m;
}

/** Watches `sync`'s uploads until the function it answers is called. */
export function tellUploadFailures(sync: AspenSync): () => void {
  const told = new Set<string>();
  return sync.uploads.subscribe(() => {
    const failing = new Set<string>();
    for (const upload of sync.uploads.all()) {
      if (
        !upload.detached ||
        upload.state.kind !== "failed" ||
        upload.state.cause === "interrupted"
      ) {
        continue;
      }
      failing.add(upload.key);
      if (!told.has(upload.key) && words !== null) {
        toast(format(words.pending.failedElsewhere, { name: upload.name }));
      }
    }
    // One tried again and failed again is told again.
    told.clear();
    for (const key of failing) {
      told.add(key);
    }
  });
}
