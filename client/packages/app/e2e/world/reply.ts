import type { Request, Route } from "@playwright/test";
import type { administration } from "./administration";
import type { lunch } from "./poll";

/** Sends a server event down the page's event stream. */
export type Publish = (event: Record<string, unknown>) => void;

/** A response other than `200` with a JSON body. */
export class Reply {
  constructor(
    readonly body: unknown,
    readonly status: number,
  ) {}
}

export const reply = (body: unknown, status: number) => new Reply(body, status);

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
}
