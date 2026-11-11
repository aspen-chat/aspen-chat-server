import type { Request, Route } from "@playwright/test";
import type { administration } from "./administration";
import type { lunch } from "./poll";

/**
 * Sends a server event down the page's event stream. The world's own also sends what happens
 * and is never kept (`ephemeral` frames), keeps every frame the client sent after `identify`,
 * and can drop the connection.
 */
export type Publish = ((event: Record<string, unknown>) => void) & {
  ephemeral?: (event: Record<string, unknown>) => void;
  sent?: Record<string, unknown>[];
  drop?: () => void;
};

/** A response other than `200` with a JSON body. */
export class Reply {
  constructor(
    readonly body: unknown,
    readonly status: number,
  ) {}
}

export const reply = (body: unknown, status: number) => new Reply(body, status);

/** A list as the server answers one: its records under `data`. */
export const listed = (records: readonly unknown[]) => ({ data: records });

/**
 * One page of `records` as a list read by cursor answers it: `limit` of them (`pageSize`
 * without one) from the place `cursor` asks for, and the cursor of the page after, `null` on
 * the last. The world's cursors are how many records come before the page.
 */
export function paged<T>(records: readonly T[], url: URL, pageSize: number) {
  const start = Number(url.searchParams.get("cursor") ?? 0);
  const end = start + Number(url.searchParams.get("limit") ?? pageSize);
  return { data: records.slice(start, end), nextCursor: end < records.length ? String(end) : null };
}

/** One route of the world: the method, the path it matches, and what it answers. */
export type WorldRoute = [string, RegExp, () => unknown];

/** The request being answered, and the world's state for this page, as the routes see them. */
export interface Asked {
  route: Route;
  request: Request;
  url: URL;
  path: string;
  poll: ReturnType<typeof lunch>;
  publish: Publish;
  admin: ReturnType<typeof administration>;
  blocks: Set<string>;
  bans: Map<string, Record<string, unknown>>;
  /** The caller's saved messages (message to save) and followed threads, as the tests change them. */
  personal: Personal;
}

/** What is the caller's own in the world: their saves, by message, and the threads they follow. */
export interface Personal {
  saves: Map<string, string>;
  follows: Set<string>;
}
