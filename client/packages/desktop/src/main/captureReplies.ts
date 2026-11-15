/**
 * What the capture helper writes on its reply pipe, one JSON object per line, each answering the
 * request whose `id` it carries (`Reply` in the crate's `main.rs`).
 *
 * This module imports nothing from Electron, so its tests run under plain Node.
 */

import type { AudioKind, CaptureKind } from "./gameCapture";

export type Reply =
  | {
      type: "kinds";
      id: number;
      kinds: CaptureKind[];
      applicationAudio: AudioKind | null;
      /** Whether the helper can capture a picture at all (a build with libobs). */
      pictures: boolean;
    }
  /**
   * `encoder` is the libobs encoder a started picture goes through, which a start alone carries;
   * `answer` the capture connection's answer, which a start and an `answer` carry.
   */
  | { type: "done"; id: number; encoder?: string; answer?: string }
  | { type: "failed"; id: number; reason: string };

/**
 * A reply that settles its request well. Whoever asked gets it whole, so a field the helper
 * sends is never dropped between the pipe and its reader.
 */
export type Answer = Exclude<Reply, { type: "failed" }>;

/**
 * The reply a line holds, or `null` for a line that is not one: text that is not JSON, or JSON
 * without a reply's `type` and `id`. The helper keeps its pipe for replies, but a line from
 * anything else that reaches it must cost a log entry, not an exception in the main process.
 */
export function parseReply(line: string): Reply | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(line);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) {
    return null;
  }
  const { type, id } = parsed as { type?: unknown; id?: unknown };
  if (typeof id !== "number" || (type !== "kinds" && type !== "done" && type !== "failed")) {
    return null;
  }
  return parsed as Reply;
}
