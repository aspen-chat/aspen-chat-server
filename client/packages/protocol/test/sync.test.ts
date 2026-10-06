import { describe, expect, it } from "vitest";
import {
  AspenClient,
  AspenSync,
  type AspenSyncOptions,
  MEMBER_SEARCH_PAGE,
  MESSAGE_AROUND_RADIUS,
  MESSAGE_PAGE_SIZE,
  MemorySessionStore,
  PRESENCE_POLL_MS,
  type Category,
  type Channel,
  type Community,
  type Message,
  type ServerEvent,
  type ServerMessage,
  type Session,
  type User,
} from "../src";

const baseUrl = "https://aspen.test";

function id(n: number): string {
  return `0190f0a0-0000-7000-8000-${n.toString(16).padStart(12, "0")}`;
}

const me: User = {
  id: id(1),
  name: "kate",
  icon: null,
  onlineStatus: "online",
  bot: false,
  system: false,
  botPublic: false,
};
const bob: User = {
  id: id(2),
  name: "bob",
  icon: null,
  onlineStatus: "offline",
  bot: false,
  system: false,
  botPublic: false,
};
const aspen: Community = { id: id(10), name: "Aspen", icon: null };
const general: Channel = {
  id: id(20),
  community: aspen.id,
  parentCategory: null,
  name: "general",
  sortIndex: 0,
  ty: "text",
  replyCount: 0,
  recipients: [],
};

function message(n: number, author = me.id): Message {
  return {
    id: id(1000 + n),
    channelId: general.id,
    author,
    content: `message ${String(n)}`,
    timestamp: "2026-09-25T12:00:00Z",
    editedAt: null,
    attachments: [],
    linkPreviews: [],
    kind: "standard",
    poll: null,
    mentions: { users: [], roles: [], everyone: false },
    linkedMessages: [],
    alteredBy: [],
  };
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": status >= 400 ? "application/problem+json" : "application/json" },
  });
}

function bootstrapResponses(): Record<string, (url: URL) => Response> {
  return {
    "/api/v1/users/@me": () => json(me),
    "/api/v1/users/@me/preferences": () => json({ values: {}, updatedAt: null }),
    "/api/v1/users/@me/admin": () => json({ permissions: [], roles: [], inclusions: [] }),
    "/api/v1/users/@me/blocks": () => json({ data: [], included: { users: [] } }),
    "/api/v1/plugins": () => json([]),
    "/api/v1/users/@me/held-messages": () => json([]),
    "/api/v1/users/statuses": (url) =>
      json(
        (url.searchParams.get("ids") ?? "")
          .split(",")
          .filter((id) => id !== "")
          .map((id) => ({ id, onlineStatus: id === me.id ? "online" : "offline" })),
      ),
    "/api/v1/users/@me/communities": (url) => {
      expect(url.searchParams.get("include")).toBe(
        "channels,categories,members,voice,readStates,mutes,collapses,roles,notifications,emoji",
      );
      return json({
        data: [aspen],
        included: {
          channels: [general],
          categories: [],
          users: [me],
          userCommunities: [{ community: aspen.id, user: me.id, sortIndex: 0 }],
          readStates: [
            { channel: general.id, lastRead: id(1000), lastMessage: id(1002), mentions: 0 },
          ],
        },
      });
    },
    "/api/v1/users/@me/dms": (url) => {
      expect(url.searchParams.get("include")).toBe("users,readStates,mutes,notifications,voice");
      return json({ data: [], included: { users: [] } });
    },
  };
}

/** Routes requests by pathname; the handler may inspect the query string and the request. */
type Route = (url: URL, request: Request) => Response | Promise<Response>;

function routedFetch(routes: Record<string, Route>) {
  const calls: URL[] = [];
  const fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const request = new Request(input, init);
    const url = new URL(request.url);
    calls.push(url);
    const handler = routes[decodeURIComponent(url.pathname)];
    if (handler === undefined) {
      throw new Error(`unexpected request ${url.toString()}`);
    }
    return Promise.resolve(handler(url, request));
  };
  return { fetch, calls };
}

class FakeSocket {
  static instances: FakeSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((m: { data: unknown }) => void) | null = null;
  onclose: ((e: { code: number; reason: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  readonly sent: unknown[] = [];

  constructor(readonly url: string) {
    FakeSocket.instances.push(this);
  }

  send(data: string): void {
    this.sent.push(JSON.parse(data));
  }

  close(): void {
    this.onclose?.({ code: 1000, reason: "" });
  }

  frame(frame: ServerMessage): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
}

/** Lets fetches, body parsing, and the stream's async connect step run to completion. */
async function settle(): Promise<void> {
  for (let i = 0; i < 4; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

function liveSession(): Session {
  return {
    userId: me.id,
    refreshToken: "refresh",
    sessionToken: "session",
    sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
  };
}

function makeSync(
  routes: Record<string, Route>,
  now = () => 0,
  extra: Partial<AspenSyncOptions> = {},
) {
  FakeSocket.instances = [];
  const store = new MemorySessionStore();
  store.save(liveSession());
  const { fetch, calls } = routedFetch(routes);
  const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
  const sync = new AspenSync({
    client,
    WebSocket: FakeSocket as unknown as typeof WebSocket,
    now,
    ...extra,
  });
  return { sync, calls };
}

/** Runs `start()` through the bootstrap and the stream's `ready`. */
async function goLive(sync: AspenSync): Promise<FakeSocket> {
  sync.start();
  expect(sync.status).toBe("bootstrapping");
  await settle();
  // The cache is usable as soon as the bootstrap lands, before the stream is up.
  expect(sync.status).toBe("connecting");
  const socket = FakeSocket.instances[0];
  if (socket === undefined) {
    throw new Error("stream did not connect after bootstrap");
  }
  socket.onopen?.();
  socket.frame({ type: "ready", userId: me.id, resumed: false });
  expect(sync.status).toBe("live");
  return socket;
}

describe("AspenSync", () => {
  it("reports activity at most once a minute, and recent activity once the stream is up", async () => {
    let now = 0;
    const { sync } = makeSync(bootstrapResponses(), () => now);
    // Activity before the stream is up is reported when it comes up.
    sync.noteActivity();
    const socket = await goLive(sync);
    const activity = () => socket.sent.filter((f) => (f as { type: string }).type === "activity");
    expect(activity()).toHaveLength(1);
    now = 30_000;
    sync.noteActivity();
    expect(activity()).toHaveLength(1);
    now = 60_000;
    sync.noteActivity();
    expect(activity()).toHaveLength(2);
    // A reconnect long after the last activity does not claim the user is active.
    now = 200_000;
    socket.close();
    await settle();
    const again = FakeSocket.instances[1];
    if (again === undefined) {
      throw new Error("no reconnect attempt");
    }
    again.onopen?.();
    again.frame({ type: "ready", userId: me.id, resumed: true });
    expect(again.sent.filter((f) => (f as { type: string }).type === "activity")).toHaveLength(0);
    sync.stop();
  });

  it("keeps a shown channel's online count current with the presence poll", async () => {
    let online = 3;
    const presencePolls: (() => void)[] = [];
    const { sync, calls } = makeSync(
      {
        ...bootstrapResponses(),
        [`/api/v1/channels/${general.id}/presence`]: () => json({ online }),
      },
      () => 0,
      {
        setTimeout: ((handler: () => void, ms?: number) => {
          if (ms === PRESENCE_POLL_MS) {
            presencePolls.push(handler);
          }
          return 0;
        }) as typeof setTimeout,
      },
    );
    await goLive(sync);
    await settle();
    const reads = () =>
      calls.filter((u) => u.pathname === `/api/v1/channels/${general.id}/presence`).length;
    expect(reads()).toBe(0);

    const unwatch = sync.watchChannelPresence(general.id);
    // A second place showing the same channel reads nothing more.
    const unwatchAgain = sync.watchChannelPresence(general.id);
    await settle();
    expect(reads()).toBe(1);
    expect(sync.store.channelOnline(general.id)).toBe(3);

    online = 5;
    presencePolls.shift()?.();
    await settle();
    expect(reads()).toBe(2);
    expect(sync.store.channelOnline(general.id)).toBe(5);

    unwatch();
    unwatchAgain();
    presencePolls.shift()?.();
    await settle();
    expect(reads()).toBe(2);
    sync.stop();
  });

  it("bootstraps from REST before connecting the stream, then applies events", async () => {
    const { sync, calls } = makeSync(bootstrapResponses());
    expect(FakeSocket.instances).toHaveLength(0);
    const socket = await goLive(sync);
    expect(calls.map((u) => u.pathname)).toEqual([
      "/api/v1/users/%40me",
      "/api/v1/users/%40me/communities",
      "/api/v1/users/@me/dms",
      "/api/v1/users/@me/admin",
      "/api/v1/users/@me/blocks",
      "/api/v1/plugins",
      "/api/v1/users/@me/held-messages",
      "/api/v1/users/%40me/preferences",
      "/api/v1/users/statuses",
    ]);
    expect(sync.store.communities()).toEqual([aspen]);
    expect(sync.store.channels(aspen.id)).toEqual([general]);

    socket.frame({
      type: "event",
      sequence: 1,
      event: { serverEvent: "community", type: "update", id: aspen.id, name: "Aspen!" },
    });
    expect(sync.store.community(aspen.id)?.name).toBe("Aspen!");
  });

  it("reports a failed bootstrap and can be started again", async () => {
    let attempts = 0;
    const { sync } = makeSync({
      ...bootstrapResponses(),
      "/api/v1/users/@me": () => {
        attempts += 1;
        return attempts === 1
          ? json({ code: "internal", title: "boom", status: 500 }, 500)
          : json(me);
      },
    });
    sync.start();
    await settle();
    expect(sync.status).toBe("failed");
    expect(sync.lastError?.code).toBe("internal");
    expect(FakeSocket.instances).toHaveLength(0);
    await goLive(sync);
  });

  it("holds events during a resync and applies them after the re-read", async () => {
    let listings = 0;
    let release: () => void = () => undefined;
    const { sync } = makeSync({
      ...bootstrapResponses(),
      "/api/v1/users/@me/communities": () => {
        listings += 1;
        return json({
          data: [aspen],
          included: {
            channels: [general],
            categories: [],
            users: [me],
            userCommunities: [{ community: aspen.id, user: me.id, sortIndex: 0 }],
          },
        });
      },
      "/api/v1/users/@me": () => {
        if (listings === 0) {
          return json(me);
        }
        // Second bootstrap: stall until the test releases it.
        return new Response(
          new ReadableStream({
            start(controller) {
              release = () => {
                controller.enqueue(new TextEncoder().encode(JSON.stringify(me)));
                controller.close();
              };
            },
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        );
      },
    });
    const socket = await goLive(sync);
    socket.frame({
      type: "event",
      sequence: 1,
      event: { serverEvent: "message", type: "delete", id: id(5) },
    });

    // Drop and come back without the server being able to resume.
    socket.close();
    await settle();
    expect(sync.status).toBe("reconnecting");
    const again = FakeSocket.instances[1];
    if (again === undefined) {
      throw new Error("no reconnect attempt");
    }
    again.onopen?.();
    again.frame({ type: "ready", userId: me.id, resumed: false });
    expect(sync.status).toBe("resyncing");

    // An event arriving mid-resync is held, not applied.
    again.frame({
      type: "event",
      sequence: 9,
      event: { serverEvent: "community", type: "update", id: aspen.id, name: "Renamed" },
    });
    expect(sync.store.community(aspen.id)?.name).toBe("Aspen");

    release();
    await settle();
    await settle();
    expect(sync.status).toBe("live");
    expect(sync.store.community(aspen.id)?.name).toBe("Renamed");
  });

  it("redoes a bootstrap that took longer than the replay window", async () => {
    let clock = 0;
    let listings = 0;
    const { sync } = makeSync(
      {
        ...bootstrapResponses(),
        "/api/v1/users/@me/communities": () => {
          listings += 1;
          clock += listings === 1 ? 55_000 : 0;
          return json({ data: [aspen], included: {} });
        },
      },
      () => clock,
    );
    sync.start();
    await settle();
    const socket = FakeSocket.instances[0];
    if (socket === undefined) {
      throw new Error("stream did not connect");
    }
    socket.onopen?.();
    socket.frame({ type: "ready", userId: me.id, resumed: false });
    expect(sync.status).toBe("resyncing");
    await settle();
    expect(listings).toBe(2);
    expect(sync.status).toBe("live");
  });

  it("loads message windows with authors, attachments, and polls sideloaded", async () => {
    const page = Array.from({ length: MESSAGE_PAGE_SIZE }, (_, i) => message(100 + i, bob.id));
    const { sync, calls } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/messages`]: (url) => {
        expect(url.searchParams.get("include")).toBe(
          "authors,memberships,attachments,polls,threads,echoes,reactions,linked,warnings,annotations",
        );
        if (url.searchParams.get("before") !== null) {
          return json({ data: [message(50)], included: { users: [], attachments: [] } });
        }
        return json({ data: page, included: { users: [bob], attachments: [] } });
      },
    });
    await goLive(sync);
    await sync.loadLatest(general.id);
    expect(sync.store.user(bob.id)).toEqual(bob);
    expect(sync.store.messages(general.id)).toMatchObject({ hasOlder: true, atLatest: true });
    expect(sync.store.messages(general.id)?.ids).toHaveLength(MESSAGE_PAGE_SIZE);

    await sync.loadOlder(general.id);
    const window = sync.store.messages(general.id);
    expect(window?.ids[0]).toBe(message(50).id);
    expect(window?.hasOlder).toBe(false);
    const olderRead = calls.find((u) => u.searchParams.has("before"));
    expect(olderRead?.searchParams.get("before")).toBe(message(100).id);
  });

  it("notes the roles of a window's authors without making them its member sample", async () => {
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/messages`]: () =>
        json({
          data: [message(1, bob.id)],
          included: {
            users: [bob],
            userCommunities: [{ community: aspen.id, user: bob.id, roles: [id(41)] }],
          },
        }),
    });
    await goLive(sync);
    await sync.loadLatest(general.id);
    expect(sync.store.memberRoles(aspen.id, bob.id)).toEqual([id(41)]);
    expect(sync.store.members(aspen.id).map((u) => u.id)).toEqual([me.id]);
    sync.stop();
  });

  it("reads the member sample again when roles shown apart change, and only then", async () => {
    const role = (n: number, hoist: boolean) => ({
      id: id(n),
      community: aspen.id,
      name: `role ${String(n)}`,
      position: n - 40,
      permissions: [],
      everyone: n === 40,
      hoist,
    });
    let sampleReads = 0;
    const { sync } = makeSync(
      {
        ...bootstrapResponses(),
        "/api/v1/users/@me/communities": () =>
          json({
            data: [aspen],
            included: {
              channels: [general],
              users: [me, bob],
              userCommunities: [
                { community: aspen.id, user: me.id, sortIndex: 0 },
                { community: aspen.id, user: bob.id, roles: [] },
              ],
              roles: [role(40, false), role(41, true), role(42, false)],
            },
          }),
        [`/api/v1/communities/${aspen.id}/members`]: () => {
          sampleReads += 1;
          return json({
            data: [me, bob],
            included: {
              userCommunities: [
                { community: aspen.id, user: me.id },
                { community: aspen.id, user: bob.id, roles: [id(41)] },
              ],
            },
          });
        },
      },
      () => 0,
      { random: () => 0 },
    );
    const socket = await goLive(sync);
    let sequence = 0;
    const hear = async (event: ServerEvent) => {
      sequence += 1;
      socket.frame({ type: "event", sequence, event });
      await settle();
    };
    await hear({ serverEvent: "role", type: "update", id: id(42), name: "renamed" });
    await hear({ serverEvent: "role", type: "update", id: id(42), position: 3 });
    expect(sampleReads).toBe(0);
    await hear({ serverEvent: "role", type: "update", id: id(42), hoist: true });
    expect(sampleReads).toBe(1);
    await hear({ serverEvent: "role", type: "update", id: id(41), position: 4 });
    expect(sampleReads).toBe(2);
    await hear({
      serverEvent: "userCommunity",
      type: "update",
      community: aspen.id,
      user: bob.id,
      roles: [id(41)],
    });
    // The sample already said Bob holds that role, so nothing changed.
    expect(sampleReads).toBe(2);
    await hear({
      serverEvent: "userCommunity",
      type: "update",
      community: aspen.id,
      user: bob.id,
      roles: [id(41), id(42)],
    });
    expect(sampleReads).toBe(3);
    sync.stop();
  });

  it("extends a window forwards and reaches the latest on a short page", async () => {
    const anchor = message(500);
    const around = [message(499), anchor, message(501)];
    const newer = [message(502), message(503)];
    const { sync, calls } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/messages`]: (url) =>
        url.searchParams.has("after")
          ? json({ data: newer, included: { users: [], attachments: [] } })
          : json({ data: around, included: { users: [], attachments: [] } }),
    });
    await goLive(sync);
    await sync.loadAround(general.id, anchor.id);
    // Both sides came back short of the radius, so the window already is at the latest.
    expect(sync.store.messages(general.id)?.atLatest).toBe(true);
    sync.store.replaceWindow(general.id, around, { hasOlder: false, atLatest: false });
    await sync.loadNewer(general.id);
    expect(calls.find((u) => u.searchParams.has("after"))?.searchParams.get("after")).toBe(
      message(501).id,
    );
    expect(sync.store.messages(general.id)).toMatchObject({ atLatest: true });
    expect(sync.store.messages(general.id)?.ids.at(-1)).toBe(message(503).id);
  });

  it("marks a window loaded around an old message as not at the latest", async () => {
    const anchor = message(500);
    const around = [
      ...Array.from({ length: MESSAGE_AROUND_RADIUS }, (_, i) => message(400 + i)),
      anchor,
      ...Array.from({ length: MESSAGE_AROUND_RADIUS }, (_, i) => message(501 + i)),
    ];
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/messages`]: (url) => {
        expect(url.searchParams.get("around")).toBe(anchor.id);
        return json({ data: around, included: { users: [], attachments: [] } });
      },
    });
    await goLive(sync);
    await sync.loadAround(general.id, anchor.id);
    expect(sync.store.messages(general.id)).toMatchObject({ hasOlder: true, atLatest: false });
  });

  it("fetches an unknown author once when a message event names one", async () => {
    let userReads = 0;
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/users/${bob.id}`]: () => {
        userReads += 1;
        return json(bob);
      },
    });
    const socket = await goLive(sync);
    socket.frame({
      type: "event",
      sequence: 1,
      event: { serverEvent: "message", type: "create", ...message(1, bob.id) },
    });
    socket.frame({
      type: "event",
      sequence: 2,
      event: { serverEvent: "message", type: "create", ...message(2, bob.id) },
    });
    await settle();
    expect(userReads).toBe(1);
    expect(sync.store.user(bob.id)).toEqual(bob);
  });

  it("creates, lists, and revokes invites through the store", async () => {
    const invite = {
      code: "abc123",
      community: aspen.id,
      createdBy: me.id,
      createdAt: "2026-09-25T12:00:00Z",
      expiresAt: null,
    };
    let revoked = false;
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/communities/${aspen.id}/invites`]: (url) =>
        url.searchParams.size === 0 && revoked ? json([]) : json([invite]),
      "/api/v1/invites/abc123": () => {
        revoked = true;
        return new Response(null, { status: 204 });
      },
    });
    await goLive(sync);
    await sync.loadInvites(aspen.id);
    expect(sync.store.invites(aspen.id)).toEqual([invite]);
    await sync.revokeInvite("abc123");
    expect(sync.store.invites(aspen.id)).toEqual([]);
  });

  it("looks up an invite with its community and joins", async () => {
    const cedar: Community = { id: id(12), name: "Cedar", icon: null };
    const lobby: Channel = { ...general, id: id(40), community: cedar.id, name: "lobby" };
    const invite = {
      code: "join-me",
      community: cedar.id,
      createdBy: bob.id,
      createdAt: "2026-09-25T12:00:00Z",
      expiresAt: "2026-09-25T13:00:00Z",
    };
    const { sync } = makeSync(
      {
        ...bootstrapResponses(),
        "/api/v1/invites/join-me": (url) => {
          expect(url.searchParams.get("include")).toBe("community");
          return json({ data: invite, included: { communities: [cedar] } });
        },
        [`/api/v1/communities/${cedar.id}/members/@me`]: () =>
          json({ community: cedar.id, user: me.id, sortIndex: 0 }, 201),
        [`/api/v1/communities/${cedar.id}`]: (url) => {
          expect(url.searchParams.get("include")).toBe(
            "channels,categories,members,voice,readStates,mutes,collapses,roles,notifications,emoji",
          );
          return json({
            data: cedar,
            included: {
              channels: [lobby],
              categories: [],
              users: [me, bob],
              userCommunities: [
                { community: cedar.id, user: me.id, sortIndex: 0 },
                { community: cedar.id, user: bob.id, sortIndex: 0 },
              ],
            },
          });
        },
      },
      () => Date.parse("2026-09-25T12:30:00Z"),
    );
    await goLive(sync);
    const lookup = await sync.lookupInvite("join-me");
    expect(lookup).toEqual({ invite, community: cedar, expired: false, member: false });
    await sync.joinCommunity(cedar.id, "join-me");
    expect(sync.store.communities().map((c) => c.name)).toEqual(["Aspen", "Cedar"]);
    expect(sync.store.channels(cedar.id)).toEqual([lobby]);
    expect(sync.store.memberIds(cedar.id)).toEqual([me.id, bob.id]);
    expect((await sync.lookupInvite("join-me")).member).toBe(true);
  });

  it("creates a community, joins it locally, and loads its channels", async () => {
    const cedar: Community = { id: id(12), name: "Cedar", icon: null };
    const lobby: Channel = { ...general, id: id(40), community: cedar.id, name: "lobby" };
    const { sync, calls } = makeSync({
      ...bootstrapResponses(),
      "/api/v1/communities": () => json(cedar, 201),
      [`/api/v1/communities/${cedar.id}`]: () =>
        json({
          data: cedar,
          included: {
            channels: [lobby],
            categories: [],
            users: [me],
            userCommunities: [{ community: cedar.id, user: me.id, sortIndex: 0 }],
          },
        }),
    });
    await goLive(sync);
    expect(await sync.createCommunity("Cedar")).toEqual(cedar);
    expect(sync.store.communities().map((c) => c.name)).toEqual(["Aspen", "Cedar"]);
    expect(sync.store.channels(cedar.id)).toEqual([lobby]);
    const create = calls.find((u) => u.pathname === "/api/v1/communities");
    expect(create).toBeDefined();
  });

  it("creates a channel after the community's last sort index", async () => {
    const created: Channel = {
      id: id(41),
      community: aspen.id,
      parentCategory: null,
      name: "random",
      sortIndex: 1,
      ty: "text",
      replyCount: 0,
      recipients: [],
    };
    const { sync } = makeSync({
      ...bootstrapResponses(),
      "/api/v1/channels": () => json(created, 201),
    });
    await goLive(sync);
    const channel = await sync.createChannel(aspen.id, {
      name: "random",
      ty: "text",
      parentCategory: null,
    });
    expect(channel).toEqual(created);
    expect(sync.store.channels(aspen.id).map((c) => c.name)).toEqual(["general", "random"]);
  });

  it("creates a category after the community's last sort index", async () => {
    const created: Category = { id: id(42), community: aspen.id, name: "Work", sortIndex: 0 };
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/communities/${aspen.id}/categories`]: () => json(created, 201),
    });
    await goLive(sync);
    expect(await sync.createCategory(aspen.id, "Work")).toEqual(created);
    expect(sync.store.categories(aspen.id)).toEqual([created]);
  });

  it("edits and deletes messages, updating the window at once", async () => {
    const original = message(7);
    const edited = { ...original, content: "edited" };
    let deleted = false;
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/messages`]: () =>
        json({ data: [original], included: { users: [], attachments: [] } }),
      [`/api/v1/messages/${original.id}`]: () => {
        if (deleted) {
          return new Response(null, { status: 204 });
        }
        deleted = true;
        return json(edited);
      },
    });
    const socket = await goLive(sync);
    await sync.loadLatest(general.id);
    expect(await sync.editMessage(original.id, "edited")).toEqual(edited);
    // The response is not written to the cache; the update event is what changes it.
    expect(sync.store.message(original.id)?.content).toBe(original.content);
    socket.frame({
      type: "event",
      sequence: 5,
      event: { serverEvent: "message", type: "update", id: original.id, content: "edited" },
    });
    expect(sync.store.message(original.id)?.content).toBe("edited");
    expect(sync.store.messages(general.id)?.ids).toEqual([original.id]);
    await sync.deleteMessage(original.id);
    expect(sync.store.message(original.id)).toBeUndefined();
    expect(sync.store.messages(general.id)?.ids).toEqual([]);
  });

  it("uploads an attachment in two phases and caches the record", async () => {
    const record = {
      id: id(900),
      fileName: "note.txt",
      mimeType: "text/plain",
      downloadUrl: "http://files.test/attachments/900",
    };
    const uploads: { url: string; method: string; type: string | null; body: string }[] = [];
    const reservations: unknown[] = [];
    const { sync } = makeSync(
      {
        ...bootstrapResponses(),
        "/api/v1/attachments": async (_url, request) => {
          reservations.push(await request.json());
          return json(
            {
              id: record.id,
              uploadUrl: "http://store.test/put/900?sig=1",
              expiresAt: "2026-09-25T13:00:00Z",
            },
            201,
          );
        },
        [`/api/v1/attachments/${record.id}/confirm`]: () => json(record),
      },
      undefined,
      {
        uploadFetch: async (input, init) => {
          const request = new Request(input, init);
          uploads.push({
            url: request.url,
            method: request.method,
            type: request.headers.get("content-type"),
            body: await request.text(),
          });
          return new Response(null, { status: 200 });
        },
      },
    );
    await goLive(sync);
    const file = new File(["hello"], "note.txt", { type: "text/plain" });
    expect(await sync.uploadAttachment(file)).toEqual(record);
    expect(uploads).toEqual([
      { url: "http://store.test/put/900?sig=1", method: "PUT", type: "text/plain", body: "hello" },
    ]);
    expect(sync.store.attachment(record.id)).toEqual(record);
    // A picture's size goes with its reservation; a file that is no picture has none.
    await sync.uploadAttachment(new File(["png"], "pic.png", { type: "image/png" }), {
      width: 640,
      height: 480,
    });
    expect(reservations).toEqual([
      { fileName: "note.txt", mimeType: "text/plain" },
      { fileName: "pic.png", mimeType: "image/png", width: 640, height: 480 },
    ]);
  });

  it("fetches an attachment record a message names but the cache lacks, once", async () => {
    const record = {
      id: id(901),
      fileName: "pic.png",
      mimeType: "image/png",
      downloadUrl: "http://files.test/attachments/901",
    };
    let reads = 0;
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/attachments/${record.id}`]: () => {
        reads += 1;
        return json(record);
      },
    });
    await goLive(sync);
    sync.ensureAttachment(record.id);
    sync.ensureAttachment(record.id);
    await settle();
    expect(reads).toBe(1);
    expect(sync.store.attachment(record.id)).toEqual(record);
  });

  it("adds and removes the caller's reaction, updating the cache at once", async () => {
    const target = message(3);
    let removed = false;
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/messages`]: () =>
        json({ data: [target], included: { users: [], attachments: [] } }),
      [`/api/v1/messages/${target.id}/reactions/👍/@me`]: () => {
        if (removed) {
          return new Response(null, { status: 204 });
        }
        removed = true;
        return json({ messageId: target.id, emoji: "👍", userId: me.id }, 201);
      },
    });
    await goLive(sync);
    await sync.loadLatest(general.id);
    await sync.addReaction(target.id, "👍");
    expect(sync.store.reactions(target.id).get("👍")).toEqual({
      count: 1,
      me: true,
      users: [me.id],
    });
    await sync.removeReaction(target.id, "👍");
    expect(sync.store.reactions(target.id).size).toBe(0);
  });

  it("blocks someone and reads again what the server counts without them", async () => {
    const bob = id(2);
    const target = message(3);
    const blocks: string[] = [];
    const routes = bootstrapResponses();
    const bootstrapCommunities = routes["/api/v1/users/@me/communities"];
    if (bootstrapCommunities === undefined) {
      throw new Error("the bootstrap reads the community list");
    }
    const { sync } = makeSync({
      ...routes,
      "/api/v1/users/@me/communities": (url) =>
        url.searchParams.get("include") === "readStates"
          ? json({
              data: [aspen],
              included: {
                readStates: [
                  { channel: general.id, lastRead: id(1000), lastMessage: null, mentions: 0 },
                ],
              },
            })
          : bootstrapCommunities(url),
      [`/api/v1/channels/${general.id}/messages`]: (url) =>
        url.searchParams.get("around") === null
          ? json({
              data: [target],
              included: {
                users: [],
                attachments: [],
                reactions: [
                  { messageId: target.id, emoji: "👍", count: 1, me: false, users: [bob] },
                ],
              },
            })
          : json({ data: [target], included: { reactions: [] } }),
      [`/api/v1/users/@me/blocks/${bob}`]: (_url, request) => {
        blocks.push(request.method);
        return request.method === "PUT"
          ? json({ user: bob, createdAt: "2026-09-28T12:00:00Z" }, 201)
          : new Response(null, { status: 204 });
      },
    });
    await goLive(sync);
    await sync.loadLatest(general.id);
    expect(sync.store.unread(general.id)).toBe(true);
    expect(sync.store.reactions(target.id).size).toBe(1);
    await sync.blockUser(bob);
    await settle();
    expect(sync.store.blocked(bob)).toBe(true);
    expect(sync.store.unread(general.id)).toBe(false);
    expect(sync.store.reactions(target.id).size).toBe(0);
    await sync.unblockUser(bob);
    expect(sync.store.blocked(bob)).toBe(false);
    expect(blocks).toEqual(["PUT", "DELETE"]);
  });

  it("opens polls, votes, and fetches a poll on demand with the caller's votes", async () => {
    const lunch = {
      id: id(3001),
      channelId: general.id,
      messageId: id(1001),
      createdBy: me.id,
      createdAt: "2026-09-25T12:00:00Z",
      closesAt: "2026-09-25T13:00:00Z",
      closedAt: null,
      question: "Lunch?",
      options: [{ label: "Pizza", emoji: "🍕" }, { label: "Sushi" }],
      multipleChoice: false,
      allowWriteIns: true,
      writeIns: [],
      anonymous: true,
      results: [{ count: 0 }, { count: 0 }],
    };
    const other = { ...lunch, id: id(3002), messageId: id(1002) };
    const votes: string[] = [];
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/polls`]: () => json(lunch, 201),
      [`/api/v1/polls/${lunch.id}/votes/1/@me`]: () => {
        votes.push("put");
        return json({ ...lunch, results: [{ count: 0 }, { count: 1 }] }, 201);
      },
      [`/api/v1/polls/${lunch.id}/votes/0/@me`]: () => {
        votes.push("delete");
        return new Response(null, { status: 204 });
      },
      [`/api/v1/polls/${other.id}`]: (url) => {
        expect(url.searchParams.get("include")).toBe("votes");
        return json({ data: other, included: { pollVotes: [{ poll: other.id, option: 0 }] } });
      },
    });
    await goLive(sync);
    const created = await sync.createPoll(general.id, {
      question: "Lunch?",
      options: [{ label: "Pizza", emoji: "🍕" }, { label: "Sushi" }],
      multipleChoice: false,
      anonymous: true,
      allowWriteIns: true,
      durationSeconds: 3600,
    });
    expect(created).toEqual(lunch);
    expect(sync.store.poll(lunch.id)).toEqual(lunch);

    await sync.vote(lunch.id, 1);
    expect(Array.from(sync.store.myVotes(lunch.id))).toEqual([1]);
    // The tally is left to the event the server published before answering.
    expect(sync.store.poll(lunch.id)?.results[1]?.count).toBe(0);
    await sync.unvote(lunch.id, 0);
    expect(votes).toEqual(["put", "delete"]);

    sync.ensurePoll(other.id);
    sync.ensurePoll(other.id);
    await settle();
    expect(sync.store.poll(other.id)).toEqual(other);
    expect(Array.from(sync.store.myVotes(other.id))).toEqual([0]);
  });

  it("writes in an answer, or votes for the one the poll already has, and removes it", async () => {
    const lunch = {
      id: id(3001),
      channelId: general.id,
      messageId: id(1001),
      createdBy: me.id,
      createdAt: "2026-09-25T12:00:00Z",
      closesAt: "2026-09-25T13:00:00Z",
      closedAt: null,
      question: "Lunch?",
      options: [{ label: "Pizza" }, { label: "Sushi" }],
      multipleChoice: false,
      allowWriteIns: true,
      writeIns: [],
      anonymous: true,
      results: [{ count: 0 }, { count: 0 }],
    };
    // The first answer is new; the second matches one the poll already has.
    const answers = [json({ option: 2, poll: lunch }, 201), json({ option: 0, poll: lunch }, 200)];
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/polls/${lunch.id}/write-ins`]: () => answers.shift() ?? json({}, 500),
      [`/api/v1/polls/${lunch.id}/write-ins/2`]: () => new Response(null, { status: 204 }),
    });
    await goLive(sync);
    sync.store.addPoll(lunch);

    expect(await sync.writeIn(lunch.id, "Tacos")).toBe(2);
    expect(Array.from(sync.store.myWriteIns(lunch.id))).toEqual([2]);
    expect(Array.from(sync.store.myVotes(lunch.id))).toEqual([2]);

    // The poll already had it: a vote, not a write-in of the caller's.
    expect(await sync.writeIn(lunch.id, "pizza")).toBe(0);
    expect(Array.from(sync.store.myWriteIns(lunch.id))).toEqual([2]);
    expect(Array.from(sync.store.myVotes(lunch.id))).toEqual([0]);

    await sync.removeWriteIn(lunch.id, 2);
    expect(Array.from(sync.store.myWriteIns(lunch.id))).toEqual([]);
  });

  it("searches a community's members without making them its member sample", async () => {
    const queries: URLSearchParams[] = [];
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/communities/${aspen.id}/members`]: (url) => {
        queries.push(url.searchParams);
        return json({
          data: [bob],
          included: {
            userCommunities: [{ community: aspen.id, user: bob.id, sortIndex: 0, roles: [id(41)] }],
          },
        });
      },
    });
    await goLive(sync);
    const found = await sync.searchMembers(aspen.id, "bo");
    expect(found.map((u) => u.id)).toEqual([bob.id]);
    expect(queries[0]?.get("filter[name]")).toBe("bo");
    expect(queries[0]?.get("limit")).toBe(String(MEMBER_SEARCH_PAGE));
    expect(sync.store.user(bob.id)).toEqual(bob);
    expect(sync.store.memberRoles(aspen.id, bob.id)).toEqual([id(41)]);
    // The sample is still the community read's.
    expect(sync.store.members(aspen.id).map((u) => u.id)).toEqual([me.id]);
    sync.stop();
  });

  it("learns at bootstrap what the caller may do across the deployment, and searches its users", async () => {
    const searches: string[] = [];
    const { sync } = makeSync({
      ...bootstrapResponses(),
      "/api/v1/users/@me/admin": () =>
        json({ permissions: ["viewDashboard"], roles: [], inclusions: [] }),
      "/api/v1/admin/users": (url) => {
        searches.push(url.search);
        return json([]);
      },
    });
    await goLive(sync);
    expect(Array.from(sync.store.deploymentPermissions())).toEqual(["viewDashboard"]);
    await sync.admin.adminUsers({ name: "  kate " });
    await sync.admin.adminUsers({ name: " ", sort: "-name", offset: 30, limit: 15 });
    expect(searches.map((s) => new URLSearchParams(s))).toEqual([
      new URLSearchParams({ "filter[name]": "kate" }),
      new URLSearchParams({ sort: "-name", offset: "30", limit: "15" }),
    ]);
    sync.stop();
    expect(sync.store.deploymentPermissions().size).toBe(0);
  });

  it("reports reading once for everything read meanwhile, and never backwards", async () => {
    const reports: unknown[] = [];
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/read-states/@me`]: async (_url, request) => {
        reports.push(await request.json());
        return new Response(null, { status: 204 });
      },
    });
    await goLive(sync);
    expect(sync.store.unread(general.id)).toBe(true);
    sync.markRead(general.id, id(1001));
    sync.markRead(general.id, id(1002));
    // Behind the position already recorded: nothing to report.
    sync.markRead(general.id, id(1000));
    expect(sync.store.readState(general.id)?.lastRead).toBe(id(1002));
    expect(sync.store.unread(general.id)).toBe(false);
    sync.flushReads();
    await settle();
    expect(reports).toEqual([{ lastRead: id(1002) }]);
    sync.flushReads();
    await settle();
    expect(reports).toHaveLength(1);
  });

  it("reads a channel's read state again when its newest message is deleted", async () => {
    let reads = 0;
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/read-states/@me`]: () => {
        reads += 1;
        return json({ channel: general.id, lastRead: id(1000), lastMessage: id(999), mentions: 0 });
      },
    });
    const socket = await goLive(sync);
    socket.frame({
      type: "event",
      sequence: 1,
      event: { serverEvent: "message", type: "delete", id: id(1001) },
    });
    await settle();
    expect(reads).toBe(0);
    socket.frame({
      type: "event",
      sequence: 2,
      event: { serverEvent: "message", type: "delete", id: id(1002) },
    });
    await settle();
    expect(reads).toBe(1);
    expect(sync.store.unread(general.id)).toBe(false);
  });

  it("looks up a channel it hears of but lacks, and reads its community again", async () => {
    const hidden: Channel = { ...general, id: id(25), name: "staff" };
    let communityReads = 0;
    const { sync, calls } = makeSync(
      {
        ...bootstrapResponses(),
        [`/api/v1/channels/${hidden.id}`]: () => json(hidden),
        [`/api/v1/communities/${aspen.id}`]: () => {
          communityReads += 1;
          return json({ data: aspen, included: { channels: [general, hidden] } });
        },
      },
      () => 0,
      { random: () => 0 },
    );
    const socket = await goLive(sync);
    // An override reaches the caller only when they may view its channel, before or after.
    socket.frame({
      type: "event",
      sequence: 1,
      event: {
        serverEvent: "channelOverride",
        type: "create",
        channel: hidden.id,
        role: id(40),
        allow: ["viewChannel"],
        deny: [],
      },
    });
    await settle();
    expect(calls.filter((url) => url.pathname === `/api/v1/channels/${hidden.id}`)).toHaveLength(1);
    expect(communityReads).toBe(1);
    expect(sync.store.channel(hidden.id)?.name).toBe("staff");
    // One it holds needs no lookup.
    socket.frame({
      type: "event",
      sequence: 2,
      event: { serverEvent: "channel", type: "update", id: general.id, parentCategory: null },
    });
    await settle();
    expect(communityReads).toBe(1);
  });

  it("reads a community again when told something announced about it did not happen", async () => {
    let communityReads = 0;
    const { sync } = makeSync(
      {
        ...bootstrapResponses(),
        [`/api/v1/communities/${aspen.id}`]: () => {
          communityReads += 1;
          return json({ data: aspen, included: { channels: [general] } });
        },
      },
      () => 0,
      { random: () => 0 },
    );
    const socket = await goLive(sync);
    socket.frame({
      type: "event",
      sequence: 1,
      event: { serverEvent: "communityResync", community: aspen.id },
    });
    await settle();
    expect(communityReads).toBe(1);
  });

  it("reads everything again when told something announced to the user did not happen", async () => {
    let bootstraps = 0;
    const { sync } = makeSync({
      ...bootstrapResponses(),
      "/api/v1/users/@me/admin": () => {
        bootstraps += 1;
        return json({ permissions: [], roles: [], inclusions: [] });
      },
    });
    const socket = await goLive(sync);
    expect(bootstraps).toBe(1);
    socket.frame({
      type: "event",
      sequence: 1,
      event: { serverEvent: "userResync", user: me.id },
    });
    await settle();
    expect(bootstraps).toBe(2);
    expect(sync.status).toBe("live");
  });

  it("renumbers only the communities and channels whose position changed", async () => {
    const birch: Community = { id: id(11), name: "Birch", icon: null };
    const dev: Channel = { ...general, id: id(21), name: "dev", sortIndex: 1 };
    const patched: string[] = [];
    const { sync } = makeSync({
      ...bootstrapResponses(),
      "/api/v1/users/@me/communities": () =>
        json({
          data: [aspen, birch],
          included: {
            channels: [general, dev],
            categories: [],
            users: [me],
            userCommunities: [
              { community: aspen.id, user: me.id, sortIndex: 0 },
              { community: birch.id, user: me.id, sortIndex: 0 },
            ],
          },
        }),
      [`/api/v1/communities/${aspen.id}/members/@me`]: () => {
        patched.push("aspen");
        return json({ community: aspen.id, user: me.id, sortIndex: 1 });
      },
      [`/api/v1/communities/${birch.id}/members/@me`]: () => {
        patched.push("birch");
        return json({ community: birch.id, user: me.id, sortIndex: 0 });
      },
      [`/api/v1/channels/${general.id}`]: () => {
        patched.push("general");
        return json({ ...general, sortIndex: 1 });
      },
      [`/api/v1/channels/${dev.id}`]: () => {
        patched.push("dev");
        return json({ ...dev, sortIndex: 0 });
      },
    });
    await goLive(sync);
    // Both memberships start at 0: aspen already sits at 0, so only birch needs a patch.
    await sync.reorderCommunities([aspen.id, birch.id]);
    expect(patched).toEqual(["birch"]);
    await sync.reorderCommunities([birch.id, aspen.id]);
    expect(sync.store.communities().map((c) => c.name)).toEqual(["Birch", "Aspen"]);
    expect(patched).toEqual(["birch", "birch", "aspen"]);
    await sync.arrangeChannels([dev.id, general.id], null);
    expect(sync.store.channels(aspen.id).map((c) => c.name)).toEqual(["dev", "general"]);
    expect(patched.slice(3).sort()).toEqual(["dev", "general"]);
  });

  it("moves a channel into another group when the arrangement names it there", async () => {
    const dev: Channel = { ...general, id: id(21), name: "dev", sortIndex: 1 };
    const { sync, calls } = makeSync({
      ...bootstrapResponses(),
      "/api/v1/users/@me/communities": () =>
        json({
          data: [aspen],
          included: {
            channels: [general, dev],
            categories: [{ id: id(30), community: aspen.id, name: "Work", sortIndex: 0 }],
            users: [me],
            userCommunities: [{ community: aspen.id, user: me.id, sortIndex: 0 }],
          },
        }),
      [`/api/v1/channels/${general.id}`]: () => json({ ...general, parentCategory: id(30) }),
    });
    await goLive(sync);
    // The Work category is empty; dropping `general` into it files it there at position 0.
    await sync.arrangeChannels([general.id], id(30));
    expect(sync.store.channel(general.id)?.parentCategory).toBe(id(30));
    const patch = calls.find((u) => decodeURIComponent(u.pathname).endsWith(general.id));
    expect(patch).toBeDefined();
    // Arranging the top level afterwards renumbers only `dev`, which now sits alone at 0.
    expect(sync.store.channels(aspen.id).filter((c) => c.parentCategory == null)).toHaveLength(1);
  });

  it("uploads an icon in two phases and patches a community with it", async () => {
    const icon = { id: id(7000), mimeType: "image/png", downloadUrl: "http://store/icons/x" };
    let putBody: Blob | null = null;
    const uploadFetch = (_input: RequestInfo | URL, init?: RequestInit) => {
      putBody = init?.body as Blob;
      return Promise.resolve(new Response(null, { status: 200 }));
    };
    const { sync, calls } = makeSync(
      {
        ...bootstrapResponses(),
        "/api/v1/icons": () =>
          json(
            { id: icon.id, uploadUrl: "http://store/put", expiresAt: "2030-01-01T00:00:00Z" },
            201,
          ),
        [`/api/v1/icons/${icon.id}/confirm`]: () => json(icon),
        [`/api/v1/communities/${aspen.id}`]: () => json({ ...aspen, icon: icon.id }),
      },
      () => 0,
      { uploadFetch },
    );
    await goLive(sync);
    const blob = new Blob([new Uint8Array([1, 2, 3])], { type: "image/png" });
    const uploaded = await sync.uploadIcon(blob, "image/png");
    expect(uploaded).toEqual(icon);
    expect(putBody).toBe(blob);
    expect(sync.store.icon(icon.id)).toEqual(icon);
    const updated = await sync.updateCommunity(aspen.id, { icon: icon.id });
    expect(updated.icon).toBe(icon.id);
    expect(decodeURIComponent(calls.at(-1)?.pathname ?? "")).toBe(
      `/api/v1/communities/${aspen.id}`,
    );
    // The cache waits for the event.
    expect(sync.store.community(aspen.id)?.icon).toBeNull();
    sync.store.applyEvent({
      serverEvent: "community",
      type: "update",
      id: aspen.id,
      icon: icon.id,
    });
    expect(sync.store.community(aspen.id)?.icon).toBe(icon.id);
  });

  it("sends profile changes as a merge patch and leaves the cache to the event", async () => {
    // The bootstrap reads the profile first; the patch is the next request to the same path.
    let requests = 0;
    const { sync, calls } = makeSync({
      ...bootstrapResponses(),
      "/api/v1/users/@me": () => {
        requests += 1;
        return requests === 1
          ? json(me)
          : json({ ...me, displayName: "Kate", status: { text: "hi", emoji: "👋" } });
      },
    });
    await goLive(sync);
    const updated = await sync.updateProfile({
      displayName: "Kate",
      pronouns: null,
      status: { text: "hi", emoji: "👋" },
    });
    expect(updated.displayName).toBe("Kate");
    expect(decodeURIComponent(calls.at(-1)?.pathname ?? "")).toBe("/api/v1/users/@me");
    expect(sync.store.me()?.displayName).toBeUndefined();
    sync.store.applyEvent({
      serverEvent: "user",
      type: "update",
      id: me.id,
      displayName: "Kate",
      pronouns: null,
      status: { text: "hi", emoji: "👋" },
    });
    expect(sync.store.me()?.displayName).toBe("Kate");
    expect(sync.store.me()?.status?.emoji).toBe("👋");
  });

  it("caches a sent message immediately and stops on sign-out", async () => {
    const sent = message(7);
    const { sync } = makeSync({
      ...bootstrapResponses(),
      [`/api/v1/channels/${general.id}/messages`]: (url) =>
        url.searchParams.has("include")
          ? json({ data: [], included: { users: [], attachments: [] } })
          : json(sent, 201),
    });
    await goLive(sync);
    await sync.loadLatest(general.id);
    await sync.sendMessage(general.id, "hello");
    expect(sync.store.messages(general.id)?.ids).toEqual([sent.id]);
    sync.stop();
    expect(sync.status).toBe("stopped");
    expect(sync.store.communities()).toEqual([]);
  });
});
