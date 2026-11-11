import { ApiProblemError, type Message, type MessagePage } from "@aspen/protocol";

/** What has been read of one deployment's feed, newest first. */
export interface FeedPages {
  readonly messages: readonly Message[];
  /**
   * The cursor of the page after those read; `null` when the deployment said there is none,
   * or when reading failed.
   */
  readonly next: string | null;
  /**
   * Whether nothing more is to be read: the last answer carried no cursor. Never judged by how
   * many messages a page held, since a page may be short, or full, and either may be the last.
   */
  readonly exhausted: boolean;
  /** Why the last page could not be read. */
  readonly problem: string | null;
}

/**
 * `earlier` with the page of a feed that `cursor` asks for after it (the first page without
 * one), as `read` reads it. A page that cannot be read leaves what was read as it is, with the
 * problem, and ends the feed.
 */
export async function withFeedPage(
  read: (cursor: string | undefined) => Promise<MessagePage>,
  earlier: readonly Message[],
  cursor: string | undefined,
): Promise<FeedPages> {
  try {
    const page = await read(cursor);
    return {
      messages: [...earlier, ...page.data],
      next: page.nextCursor,
      exhausted: page.nextCursor === null,
      problem: null,
    };
  } catch (error) {
    return {
      messages: earlier,
      next: null,
      exhausted: true,
      problem: error instanceof ApiProblemError ? error.message : String(error),
    };
  }
}
