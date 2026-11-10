import type { AspenSync } from "@aspen/protocol";
import { detectShell } from "@/config";
import type { Messages } from "@/i18n/messages";

/**
 * Asking before the page goes while a message the reader sent is still uploading its files
 * (`AspenSync.uploads`): a browser's page sends them, so leaving stops them and the message is
 * never posted. In a browser the tab asks, in its own words; in the desktop app the window does
 * (`packages/desktop/src/main/sendingGuard.ts`), in the reader's. A phone's app keeps sending in
 * the background, and asks nothing.
 */

/** The sessions whose uploads are watched: home, and each other deployment signed in to. */
const watched = new Set<AspenSync>();
let question: { message: string; detail: string; stay: string; quit: string } | null = null;
let told = false;

function sending(): boolean {
  for (const sync of watched) {
    if (sync.uploads.sending) {
      return true;
    }
  }
  return false;
}

function onBeforeUnload(event: BeforeUnloadEvent): void {
  if (sending()) {
    event.preventDefault();
  }
}

/** Tells the desktop window whether to ask, when that has changed, or its wording has. */
function tell(reworded = false): void {
  const now = sending();
  if (now !== told || (now && reworded)) {
    told = now;
    window.aspenDesktop?.sending.set(now ? question : null);
  }
}

/** The question the desktop window asks, in the reader's language as it is now. */
export function wordSendingGuard(m: Messages): void {
  question = {
    message: m.pending.leaveQuestion,
    detail: m.pending.leaveDetail,
    stay: m.pending.leaveStay,
    quit: m.pending.leaveQuit,
  };
  if (detectShell() === "desktop") {
    tell(true);
  }
}

/** Watches `sync`'s uploads until the function it answers is called. */
export function guardSending(sync: AspenSync): () => void {
  const shell = detectShell();
  if (shell === "mobile") {
    return () => undefined;
  }
  if (watched.size === 0 && shell === "web") {
    window.addEventListener("beforeunload", onBeforeUnload);
  }
  watched.add(sync);
  const stop =
    shell === "desktop"
      ? sync.uploads.subscribe(() => {
          tell();
        })
      : () => undefined;
  return () => {
    stop();
    watched.delete(sync);
    if (shell === "desktop") {
      tell();
    } else if (watched.size === 0) {
      window.removeEventListener("beforeunload", onBeforeUnload);
    }
  };
}
