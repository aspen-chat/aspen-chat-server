import type { Route } from "@playwright/test";
import type { administration } from "./administration";
import { HISTORY_DELAY_MS } from "./fixtures";
import type { lunch } from "./poll";
import { type Asked, type Personal, type Publish, Reply, type WorldRoute } from "./reply";
import { adminRoutes } from "./routes/admin";
import { coreRoutes } from "./routes/core";
import { messageRoutes } from "./routes/messages";

function json(route: Route, body: unknown, status = 200) {
  return route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });
}

/** Answers every API request the app makes about the world in fixtures.ts and its sub-worlds. */
export async function answer(
  route: Route,
  poll: ReturnType<typeof lunch>,
  publish: Publish,
  admin: ReturnType<typeof administration>,
  blocks: Set<string>,
  bans: Map<string, Record<string, unknown>>,
  personal: Personal,
) {
  const request = route.request();
  const url = new URL(request.url());
  const path = decodeURIComponent(url.pathname.replace(/^.*\/api\/v1/, ""));
  const method = request.method();
  const asked: Asked = {
    route,
    request,
    url,
    path,
    poll,
    publish,
    admin,
    blocks,
    bans,
    personal,
  };
  // First match wins, in this order.
  const routes: WorldRoute[] = [
    ...coreRoutes(asked),
    ...adminRoutes(asked),
    ...messageRoutes(asked),
  ];
  if (url.searchParams.has("before")) {
    await new Promise((resolve) => setTimeout(resolve, HISTORY_DELAY_MS));
  }
  const match = routes.find(([m, pattern]) => m === method && pattern.test(path));
  if (match === undefined) {
    return route.fulfill({
      status: 404,
      contentType: "application/problem+json",
      body: JSON.stringify({
        code: "notFound",
        title: `Not stubbed: ${method} ${path}`,
        status: 404,
      }),
    });
  }
  const result = match[2]();
  if (result instanceof Reply) {
    return result.body === null
      ? route.fulfill({ status: result.status })
      : json(route, result.body, result.status);
  }
  return json(route, result);
}
