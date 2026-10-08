import type { Message, RecordStore } from "@aspen/protocol";
import { useLayoutEffect, useMemo, useState, type RefObject } from "react";
import type { WindowPart } from "@/features/messages/blocked";
import { isImageType, isVideoType } from "@/features/messages/images";
import {
  estimateHeight,
  GROUP_HEIGHT_SHARE,
  groupContinuations,
  type GroupCandidate,
  type MessageContents,
  type TextMetrics,
} from "@/features/messages/messageGroups";

/** What a message's picture, name, and gap beside its column take of a row's width. */
const BESIDE_COLUMN_PX = 36 + 12 + 16;

/** The list's size and how it sets text, as grouping estimates with it. */
interface ListMetrics extends TextMetrics {
  readonly height: number;
}

/**
 * The messages among `parts` that continue the group before them (`messageGroups`): only
 * messages shown as they are, never those of a run of blocked messages, which stands between
 * the groups around it, nor the first under the New Messages line, which follows the message
 * at `lineIndex` (`-1`: the line is above them all). Worked out again whenever the parts or
 * the line change, each message keeping what it was decided to be while it stays in the
 * window, and afresh as the list (`viewport`, its text set in `content`) is resized. Nothing
 * is grouped until the list is `drawn` and measured, which is done before it is painted.
 */
export function useGroupContinuations(
  parts: readonly WindowPart[],
  lineIndex: number | null,
  store: RecordStore,
  viewport: RefObject<HTMLElement | null>,
  content: RefObject<HTMLElement | null>,
  drawn: boolean,
): ReadonlySet<string> {
  const metrics = useListMetrics(viewport, content, drawn);
  const [decisions] = useState(() => new Decisions());
  return useMemo(() => {
    if (metrics === null) {
      return NONE;
    }
    const candidates: GroupCandidate[] = [];
    let afterBreak = lineIndex === -1;
    for (const part of parts) {
      if (part.kind === "blocked") {
        afterBreak = true;
        continue;
      }
      const message = store.message(part.id);
      if (message !== undefined) {
        candidates.push({
          id: part.id,
          author: message.author,
          at: Date.parse(message.timestamp),
          alone: message.kind !== "standard" && message.kind !== "poll",
          height: estimateHeight(contentsOf(message, store), metrics),
          breakBefore: afterBreak,
        });
      }
      afterBreak = part.index === lineIndex;
    }
    const decided = decisions.under(metrics);
    const continuing = groupContinuations(candidates, metrics.height * GROUP_HEIGHT_SHARE, decided);
    // A message that left the window is decided afresh if it returns.
    const kept = new Set(candidates.map((c) => c.id));
    for (const id of decided.keys()) {
      if (!kept.has(id)) {
        decided.delete(id);
      }
    }
    return continuing;
  }, [parts, lineIndex, store, metrics, decisions]);
}

const NONE: ReadonlySet<string> = new Set();

/** What each message in the list was decided to be, under the metrics it was decided with. */
class Decisions {
  #metrics: ListMetrics | null = null;
  #decided = new Map<string, boolean>();

  /** The decisions made under `metrics`: none, when the list has been resized since. */
  under(metrics: ListMetrics): Map<string, boolean> {
    if (metrics !== this.#metrics) {
      this.#metrics = metrics;
      this.#decided = new Map();
    }
    return this.#decided;
  }
}

/** What `message` holds, as far as its height goes. */
function contentsOf(message: Message, store: RecordStore): MessageContents {
  const pictures: { width?: number; height?: number }[] = [];
  let files = 0;
  for (const id of message.attachments) {
    const attachment = store.attachment(id);
    if (attachment === undefined) {
      pictures.push({});
    } else if (isImageType(attachment.mimeType) || isVideoType(attachment.mimeType)) {
      pictures.push({
        ...(attachment.width == null ? {} : { width: attachment.width }),
        ...(attachment.height == null ? {} : { height: attachment.height }),
      });
    } else {
      files += 1;
    }
  }
  let cards = 0;
  for (const preview of message.linkPreviews) {
    if (preview.imageUrl != null && preview.title == null && preview.description == null) {
      pictures.push({
        ...(preview.imageWidth == null ? {} : { width: preview.imageWidth }),
        ...(preview.imageHeight == null ? {} : { height: preview.imageHeight }),
      });
    } else {
      cards += 1;
    }
  }
  return {
    content: message.content,
    pictures,
    files,
    cards,
    poll: message.poll != null,
    reactions: store.reactions(message.id).size > 0,
    thread: message.thread != null,
  };
}

/**
 * The list's height and text metrics, followed as it is resized; `null` until it is `drawn`
 * and measured.
 */
function useListMetrics(
  viewport: RefObject<HTMLElement | null>,
  content: RefObject<HTMLElement | null>,
  drawn: boolean,
): ListMetrics | null {
  const [metrics, setMetrics] = useState<ListMetrics | null>(null);
  useLayoutEffect(() => {
    const box = viewport.current;
    if (!drawn || box === null) {
      return;
    }
    const measure = () => {
      const text = content.current ?? box;
      const style = getComputedStyle(text);
      const fontSize = Number.parseFloat(style.fontSize) || 16;
      const lineHeight = Number.parseFloat(style.lineHeight) || fontSize * 1.5;
      const next: ListMetrics = {
        height: box.clientHeight,
        column: Math.max(1, text.clientWidth - BESIDE_COLUMN_PX),
        lineHeight,
        // An average narrow character in the sans faces is about half the text's size.
        charWidth: fontSize / 2,
      };
      setMetrics((prior) =>
        prior !== null &&
        prior.height === next.height &&
        prior.column === next.column &&
        prior.lineHeight === next.lineHeight &&
        prior.charWidth === next.charWidth
          ? prior
          : next,
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(box);
    return () => {
      observer.disconnect();
    };
  }, [viewport, content, drawn]);
  return metrics;
}
