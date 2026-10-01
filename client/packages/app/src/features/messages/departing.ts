import type { RecordStore } from "@aspen/protocol";
import { useLayoutEffect, useRef, useState, type RefObject } from "react";
import { useMotion } from "@/features/layout/motion";

/** A message deleted while shown: where it stood, and how tall it was. */
export interface Departing {
  readonly id: string;
  /** The message still shown that it came after, or `null` when it was first. */
  readonly after: string | null;
  readonly height: number;
}

/** How long after a deletion a message's leaving may still be drawn. */
const RECENT_MS = 1000;

/**
 * The messages just deleted from a list, each to be drawn for a moment as an empty space of
 * its height that closes (`DepartingSpace`), so what was below it moves up into its place
 * rather than jumping; each is forgotten (`forget`) once its space has closed. Only deletions count (`RecordStore.departedAt`): messages that leave
 * because the window moved, or the channel changed, go at once, as they do with animations off
 * or where the device asks to reduce motion. Heights are measured from the list's
 * `[data-message-id]` elements whenever the ids change and whenever `measure` is called, as
 * the list does when its content changes size.
 */
export function useDeparting(
  scroller: RefObject<HTMLElement | null>,
  ids: readonly string[],
  store: RecordStore,
): { departing: readonly Departing[]; measure: () => void; forget: (id: string) => void } {
  const motion = useMotion();
  const heights = useRef(new Map<string, number>());
  const last = useRef<readonly string[]>([]);
  const [departing, setDeparting] = useState<readonly Departing[]>([]);

  const measure = () => {
    const element = scroller.current;
    if (element === null) {
      return;
    }
    for (const node of element.querySelectorAll<HTMLElement>("[data-message-id]")) {
      const id = node.dataset.messageId;
      if (id !== undefined) {
        heights.current.set(id, node.offsetHeight);
      }
    }
  };

  // Measured in a layout effect, before the list is painted without the message, so its space
  // is drawn in the same frame the message goes.
  useLayoutEffect(() => {
    const note = () => {
      const previous = last.current;
      last.current = ids;
      const present = new Set(ids);
      const gone: Departing[] = [];
      if (!motion.off && !motion.reduced) {
        let after: string | null = null;
        for (const id of previous) {
          if (present.has(id)) {
            after = id;
            continue;
          }
          const at = store.departedAt(id);
          const height = heights.current.get(id);
          if (at !== undefined && Date.now() - at < RECENT_MS && height !== undefined) {
            gone.push({ id, after, height });
          }
        }
      }
      for (const id of previous) {
        if (!present.has(id)) {
          heights.current.delete(id);
        }
      }
      measure();
      if (gone.length > 0) {
        setDeparting((current) => [...current, ...gone]);
      }
    };
    note();
    // Measured through the ref; run again only when the ids change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ids]);

  const forget = (id: string) => {
    setDeparting((current) => current.filter((gone) => gone.id !== id));
  };

  return { departing, measure, forget };
}
