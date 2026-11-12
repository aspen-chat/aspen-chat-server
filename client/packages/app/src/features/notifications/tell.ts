/** What a system notification shows, and what clicking it does. */
export interface Shown {
  title: string;
  body: string;
  tag: string;
  open: () => void;
}

/**
 * A notification as it is described: its body may still be on its way (`messageBody`), and
 * comes as `null` when there is no longer anything to tell of (the message was deleted
 * meanwhile).
 */
export type Described = Omit<Shown, "body"> & { body: string | Promise<string | null> };

/** Where `tell` tells: the user's settings and the system's notifications. */
export interface Telling {
  /** Whether nothing may tell now: do not disturb, or the user is looking at the conversation. */
  quiet(): boolean;
  /** Plays the chime, where the settings ask for it. */
  chime(): void;
  /** Whether system notifications are on, allowed, and this app's to show. */
  shows(): boolean;
  show(shown: Shown): void;
}

/**
 * Tells the user of something: the chime, and the system's notification where it is shown.
 * `describe` is asked only once everything says a notification will be shown, so whatever it
 * reads for one is never read for nothing. A body that was waited for is shown only if the
 * notification still would be.
 */
export function tell(telling: Telling, describe: () => Described): void {
  if (telling.quiet()) {
    return;
  }
  telling.chime();
  if (!telling.shows()) {
    return;
  }
  const { body, ...rest } = describe();
  if (typeof body === "string") {
    telling.show({ ...rest, body });
    return;
  }
  void body.then((body) => {
    if (body !== null && !telling.quiet() && telling.shows()) {
      telling.show({ ...rest, body });
    }
  });
}
