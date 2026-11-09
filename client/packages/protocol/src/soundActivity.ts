/**
 * The app's sounds that are playing, or asked for and about to, which a stopped microphone waits
 * for before it is released (`browserMedia`). While a capture with echo cancellation is open,
 * Chromium mixes everything the page plays to a device, so it can take that out of what the
 * microphone hears; about a second after the last such capture closes, it takes the page's
 * outputs out of that mix, which cuts whatever they are playing for some milliseconds and
 * resumes it some decibels quieter. A sound playing then (the one that says the user left the
 * call, which starts as the call releases its microphone) is heard to click.
 */

let playing = 0;
const waiting = new Set<() => void>();

/**
 * Counts one sound as playing until the function it answers is called, which may be called
 * more than once.
 */
export function playingSound(): () => void {
  playing += 1;
  let ended = false;
  return () => {
    if (ended) {
      return;
    }
    ended = true;
    playing -= 1;
    if (playing === 0) {
      for (const wake of [...waiting]) {
        wake();
      }
    }
  };
}

/** Settles once no sound counted by `playingSound` plays, or after `limitMs` at the latest. */
export function soundsEnded(limitMs: number): Promise<void> {
  if (playing === 0) {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    const wake = () => {
      clearTimeout(limit);
      waiting.delete(wake);
      resolve();
    };
    const limit = setTimeout(wake, limitMs);
    waiting.add(wake);
  });
}
