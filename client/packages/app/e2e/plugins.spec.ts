import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import {
  bob,
  community,
  dm,
  general,
  organiserRoleWith,
  ownText,
  roadmap,
  settleAnimations,
  signInToWorld,
  type Publish,
} from "./world";

const PLUGIN = "org.example.filter";
const PRINCIPAL = "0190f0a0-0000-7000-8000-0000000009a0";

/** A plugin as `GET /plugins` gives it: one that also reads DMs, with an account. */
const filter = {
  id: PLUGIN,
  version: "1.0.0",
  name: "Word filter",
  description: "Masks the words you list.",
  author: null,
  homepage: null,
  mode: "optIn",
  dms: true,
  principal: PRINCIPAL,
  principalPermissions: ["viewChannel", "sendMessages"],
  communitySettings: [{ name: "watchWords", type: "textList", label: "watch" }],
  channelTypes: [],
  messages: {
    watch: "Words to flag",
    watched: "Mentions “%{word}”",
    watchedDetail: "This community's moderators asked to be told of this word.",
  },
};

/** Signs in to the world with the filter running, its community use answered as asked. */
async function withFilter(page: Page): Promise<{ publish: Publish; turnedOn: unknown[] }> {
  const turnedOn: unknown[] = [];
  let enabled = false;
  const publish = await signInToWorld(page, async (p) => {
    await p.route(/\/api\/v1\/plugins$/, (route) => route.fulfill({ json: [filter] }));
    await p.route(new RegExp(`/api/v1/communities/${community}/plugins(/.*)?$`), async (route) => {
      const method = route.request().method();
      const record = () => ({
        community,
        plugin: PLUGIN,
        enabled,
        settings: {},
        secretsSet: [],
      });
      if (method === "PUT") {
        turnedOn.push(route.request().postDataJSON());
        enabled = true;
        await route.fulfill({ status: 201, json: record() });
      } else {
        await route.fulfill({ json: [record()] });
      }
    });
  });
  return { publish, turnedOn };
}

/** On a phone, which shows a channel alone, goes back to the channel list. */
async function toChannelList(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
}

async function openGeneral(page: Page) {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "general", exact: true })).toBeVisible();
}

test("a plugin's note on a message shows beneath it, and says which plugin said it", async ({
  page,
}) => {
  const { publish } = await withFilter(page);
  await openGeneral(page);
  const own = page.locator("article").filter({ hasText: ownText }).last();
  await expect(own).toBeVisible();
  const id = await own.getAttribute("data-message-id");
  publish({
    serverEvent: "messageAnnotation",
    type: "create",
    id: "0190f0a0-0000-7000-8000-0000000009b0",
    message: id,
    plugin: PLUGIN,
    kind: "watched",
    severity: "notice",
    label: { key: "watched", args: { word: "lemonade" } },
    detail: { key: "watchedDetail" },
    link: null,
  });
  const notes = own.getByRole("list", { name: "Notes from this server's plugins" });
  const note = notes.getByRole("button", { name: "Mentions “lemonade”" });
  await expect(note).toBeVisible();
  await note.click();
  const popover = page.getByRole("dialog", { name: "Mentions “lemonade”" });
  await expect(popover).toContainText("From Word filter");
  await expect(popover).toContainText("asked to be told of this word");
});

test("a message a plugin changed says so, naming the plugin", async ({ page }) => {
  const { publish } = await withFilter(page);
  await openGeneral(page);
  publish({
    serverEvent: "message",
    type: "create",
    id: "0190f0a0-0000-7000-8001-0000000009c0",
    channelId: general,
    author: bob,
    content: "**** it, the van is late",
    timestamp: new Date().toISOString(),
    editedAt: null,
    linkPreviews: [],
    linkedMessages: [],
    alteredBy: [PLUGIN],
    kind: "standard",
    poll: null,
    thread: null,
    echoOf: null,
    attachments: [],
    mentions: { users: [], roles: [], everyone: false },
  });
  const changed = page.locator("article").filter({ hasText: "the van is late" });
  await expect(changed).toContainText("(changed by Word filter)");
});

test("a DM says which plugins of the server can read it", async ({ page }) => {
  await withFilter(page);
  await page.goto(`/dms/${dm}`);
  await expect(page.getByRole("note")).toContainText("Word filter can read this conversation.");
});

test("a holder of Manage plugins turns a plugin on, granting its account what it asks", async ({
  page,
  isMobile,
}) => {
  const { publish, turnedOn } = await withFilter(page);
  // Once the stream is up, which the channel's history shows.
  await openGeneral(page);
  if (isMobile) {
    // A phone shows the channel alone; the community's settings are on its channel list.
    await page.goBack();
  }
  publish(organiserRoleWith(["managePlugins"]));
  await page.getByRole("button", { name: "Community settings" }).click();
  const settings = page.getByRole("dialog");
  await settings.getByRole("tab", { name: "Plugins" }).click();
  await expect(settings.getByRole("heading", { name: /Word filter/ })).toBeVisible();
  await settings.getByRole("button", { name: "Turn on" }).click();
  await expect.poll(() => turnedOn.length).toBe(1);
  expect(turnedOn[0]).toMatchObject({ grant: ["viewChannel", "sendMessages"] });
  await expect(settings.getByLabel("Words to flag")).toBeVisible();
});

const BOARDS = "org.example.boards";
const BOARDS_PRINCIPAL = "0190f0a0-0000-7000-8000-0000000009d0";
const BOARD = "0190f0a0-0000-7000-8000-0000000009d1";
const VIEW = `/api/v1/plugins/${BOARDS}/assets/board.html`;

/** A plugin that adds a kind of channel, shown by its own page, and posts cards. */
const boards = {
  ...filter,
  id: BOARDS,
  name: "Boards",
  description: "Forum boards.",
  dms: false,
  principal: BOARDS_PRINCIPAL,
  communitySettings: [],
  channelTypes: [{ pluginType: `${BOARDS}:board`, name: "Board", glyph: "board", view: VIEW }],
  messages: {
    meetup: "Meetup",
    when: "When",
    going: "Going",
    host: "Host",
    details: "Details",
    agenda: "The agenda",
    rsvp: "I'm going",
    remind: "Starts in an hour: %{title}",
  },
};

/**
 * The page a channel of the kind shows, standing in for a plugin's own: it says `ready`, and on
 * `hello` names what it shows and whom for, calls a route of its plugin, tries to reach beyond
 * its routes, asks who someone is, and shows the plugin's events, each where a test can read it.
 */
const PAGE = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Board</title></head>
<body>
<p id="hello"></p><p id="answer"></p><p id="escape"></p><p id="people"></p><p id="events"></p>
<p id="window"></p>
<script>
const out = (id, text) => { document.getElementById(id).textContent = text; };
let next = 0;
let port = null;
const pending = new Map();
const ask = (message) => new Promise((resolve) => {
  next += 1;
  pending.set(next, resolve);
  port.postMessage({ aspen: 1, ...message, id: next });
});
addEventListener("message", async (event) => {
  const message = event.data;
  if (!message || message.aspen !== 1) return;
  if (message.type === "goTo") {
    location.href = "https://elsewhere.test/page";
  } else if (message.type === "hello" && port === null && event.ports.length === 1) {
    port = event.ports[0];
    port.onmessage = (portEvent) => {
      const reply = portEvent.data;
      if (reply.type === "response" || reply.type === "users") {
        pending.get(reply.id)?.(reply);
      } else if (reply.type === "event") {
        out("events", reply.kind + " " + JSON.stringify(reply.payload));
      }
    };
    const context = message.context;
    out("hello", context.channelName + " for " + context.user.name + ", " + context.dir);
    // The frame's window is not the bridge: a request there goes unanswered.
    parent.postMessage({ aspen: 1, type: "request", id: 99, method: "GET", path: "window" }, "*");
    const answer = await ask({ type: "request", method: "GET", path: "boards/" + context.channel + "/posts" });
    out("answer", answer.status + " " + answer.body);
    const escape = await ask({ type: "request", method: "DELETE", path: "../../users/@me" });
    out("escape", "escape " + escape.status);
    const people = await ask({ type: "users", ids: [context.user.id] });
    out("people", people.users.map((user) => user.displayName).join(", "));
  }
});
</script></body></html>`;

/** The policy the deployment serves a view's files with. */
const VIEW_POLICY =
  "sandbox allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox; " +
  "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; " +
  "connect-src 'none'; form-action 'none'; base-uri 'none'";

/**
 * Signs in to the world with the boards plugin running, its view served, and its routes
 * answered; returns what its routes and card buttons were asked.
 */
async function withBoards(page: Page): Promise<{ publish: Publish; asked: string[] }> {
  const asked: string[] = [];
  const publish = await signInToWorld(page, async (p) => {
    await p.route(/\/api\/v1\/plugins$/, (route) => route.fulfill({ json: [boards] }));
    await p.route(`**${VIEW}`, (route) =>
      route.fulfill({
        body: PAGE,
        contentType: "text/html; charset=utf-8",
        headers: { "content-security-policy": VIEW_POLICY },
      }),
    );
    await p.route(new RegExp(`/api/v1/plugins/${BOARDS}/routes/`), async (route) => {
      const url = new URL(route.request().url());
      asked.push(`${route.request().method()} ${url.pathname}`);
      await route.fulfill({ json: [{ title: "Welcome" }] });
    });
    await p.route(/\/api\/v1\/messages\/[^/]+\/card\/buttons\/[^/]+$/, async (route) => {
      asked.push(`${route.request().method()} ${new URL(route.request().url()).pathname}`);
      await route.fulfill({ status: 204 });
    });
  });
  return { publish, asked };
}

/** A channel of a plugin's kind, as the stream announces it. */
function addBoard(publish: Publish, pluginType: string, name = "announcements") {
  publish({
    serverEvent: "channel",
    type: "create",
    id: BOARD,
    parentChannel: null,
    starterMessage: null,
    replyCount: 0,
    lastReplyAt: null,
    recipients: [],
    parentCategory: null,
    community,
    name,
    sortIndex: 5,
    ty: "plugin",
    pluginType,
  });
}

/** Opens the board from the channel list, going back to the list first on a phone. */
async function openBoard(page: Page, name = "announcements") {
  await toChannelList(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText(name, { exact: true })
    .click();
  await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
}

test("a channel of a plugin's kind shows its page, which reaches the app only by the bridge", async ({
  page,
}) => {
  const { publish, asked } = await withBoards(page);
  await openGeneral(page);
  addBoard(publish, `${BOARDS}:board`);
  await openBoard(page);
  const view = page.frameLocator('iframe[title="announcements, shown by the Boards plugin"]');
  await expect(view.locator("#hello")).toHaveText("announcements for kate, ltr");
  await expect(view.locator("#answer")).toHaveText('200 [{"title":"Welcome"}]');
  // A path beyond the plugin's routes is answered without being sent.
  await expect(view.locator("#escape")).toHaveText("escape 400");
  await expect(view.locator("#people")).toHaveText("Kate");
  expect(asked).toContain(`GET /api/v1/plugins/${BOARDS}/routes/boards/${BOARD}/posts`);
  expect(asked.every((call) => call.includes(`/plugins/${BOARDS}/routes/`))).toBe(true);
  // Asked on the frame's window rather than the port, nothing is answered.
  expect(asked).not.toContain(`GET /api/v1/plugins/${BOARDS}/routes/window`);

  // The plugin's events for the channel reach the page; another channel's and another
  // plugin's do not.
  publish({
    serverEvent: "pluginEvent",
    plugin: BOARDS,
    channel: roadmap,
    kind: "elsewhere",
    payload: {},
  });
  publish({
    serverEvent: "pluginEvent",
    plugin: PLUGIN,
    channel: BOARD,
    kind: "other",
    payload: {},
  });
  publish({
    serverEvent: "pluginEvent",
    plugin: BOARDS,
    channel: BOARD,
    kind: "changed",
    payload: { posts: 2 },
  });
  await expect(view.locator("#events")).toHaveText('changed {"posts":2}');

  await settleAnimations(page);
  const audit = await new AxeBuilder({ page }).analyze();
  expect(audit.violations.map((v) => v.id)).toEqual([]);
});

/**
 * A page elsewhere that a view's link leads to, which asks the bridge on the frame's window at
 * once, before its own load, and again after it.
 */
const ELSEWHERE = "https://elsewhere.test/page";
const ELSEWHERE_PAGE = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Elsewhere</title></head>
<body><p id="here">elsewhere</p>
<script>
const attempt = () =>
  parent.postMessage({ aspen: 1, type: "request", id: 1, method: "DELETE", path: "boards/all" }, "*");
parent.postMessage({ aspen: 1, type: "ready" }, "*");
attempt();
addEventListener("load", () => setTimeout(attempt, 100));
</script></body></html>`;

test("a view that goes to another page loses the bridge until it is loaded again", async ({
  page,
}) => {
  const { publish, asked } = await withBoards(page);
  await page.route(ELSEWHERE, (route) =>
    route.fulfill({ body: ELSEWHERE_PAGE, contentType: "text/html; charset=utf-8" }),
  );
  await openGeneral(page);
  addBoard(publish, `${BOARDS}:board`);
  await openBoard(page);
  const title = 'iframe[title="announcements, shown by the Boards plugin"]';
  const view = page.frameLocator(title);
  await expect(view.locator("#answer")).toHaveText('200 [{"title":"Welcome"}]');
  const before = asked.length;

  // The view's page sends its frame elsewhere, as a link in it would.
  await page.locator(title).evaluate((frame: HTMLIFrameElement) => {
    frame.contentWindow?.postMessage({ aspen: 1, type: "goTo" }, "*");
  });
  const reload = page.getByRole("button", { name: "Load the view again" });
  await expect(reload).toBeVisible();
  await page.waitForTimeout(300);
  expect(asked.slice(before)).toEqual([]);

  await reload.click();
  await expect(view.locator("#answer")).toHaveText('200 [{"title":"Welcome"}]');
});

test("a channel of a kind no plugin here declares says it needs one", async ({ page }) => {
  const { publish } = await withBoards(page);
  await openGeneral(page);
  addBoard(publish, "org.example.gone:board", "old board");
  await openBoard(page, "old board");
  await expect(
    page.getByText("This channel is shown by a plugin this server does not run here."),
  ).toBeVisible();
  await expect(page.locator("iframe")).toHaveCount(0);
});

test("a card on a plugin's message shows its fields, and its button calls the plugin", async ({
  page,
}) => {
  const { publish, asked } = await withBoards(page);
  await openGeneral(page);
  const id = "0190f0a0-0000-7000-8001-0000000009e0";
  const at = "2026-11-02T18:30:00Z";
  publish({
    serverEvent: "message",
    type: "create",
    id,
    channelId: general,
    author: BOARDS_PRINCIPAL,
    content: "A new event",
    timestamp: new Date().toISOString(),
    editedAt: null,
    linkPreviews: [],
    linkedMessages: [],
    alteredBy: [],
    kind: "standard",
    poll: null,
    thread: null,
    echoOf: null,
    attachments: [],
    mentions: { users: [], roles: [], everyone: false },
    card: {
      plugin: BOARDS,
      title: { key: "meetup" },
      fields: [
        { label: { key: "when" }, value: { type: "time", at } },
        { label: { key: "going" }, value: { type: "count", count: 1200 } },
        { label: { key: "host" }, value: { type: "person", user: bob } },
        {
          label: { key: "details" },
          value: { type: "link", url: "https://example.org/agenda", text: { key: "agenda" } },
        },
      ],
      buttons: [{ id: "rsvp", label: { key: "rsvp" }, style: "primary" }],
    },
  });
  const card = page.getByRole("region", { name: "Meetup" });
  await expect(card).toBeVisible();
  await expect(card.locator(`time[datetime="${at}"]`)).toBeVisible();
  await expect(card).toContainText("1,200");
  await expect(card).toContainText("Bob With A Rather Long Display Name");
  await expect(card.getByRole("link", { name: "The agenda" })).toHaveAttribute(
    "href",
    "https://example.org/agenda",
  );
  await card.getByRole("button", { name: "I'm going" }).click();
  await expect.poll(() => asked).toContain(`POST /api/v1/messages/${id}/card/buttons/rsvp`);

  await settleAnimations(page);
  const audit = await new AxeBuilder({ page }).include("main").analyze();
  expect(audit.violations.map((v) => v.id)).toEqual([]);
});

test("a plugin's notice shows a system notification naming the plugin", async ({ page }) => {
  await page.addInitScript(() => {
    const shown: { title: string; body: string }[] = [];
    Object.assign(window, { shownNotices: shown });
    class StandIn {
      static permission = "granted";
      static requestPermission() {
        return Promise.resolve("granted");
      }
      onclick: (() => void) | null = null;
      constructor(title: string, options: { body: string }) {
        shown.push({ title, body: options.body });
      }
      close() {
        return undefined;
      }
    }
    Object.defineProperty(window, "Notification", { value: StandIn });
    HTMLMediaElement.prototype.play = () => Promise.resolve();
  });
  const { publish } = await withBoards(page);
  await openGeneral(page);
  await toChannelList(page);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByText("Show system notifications", { exact: true }).click();
  await expect(settings.getByRole("checkbox", { name: /Show system notifications/ })).toBeChecked();
  await page.keyboard.press("Escape");
  publish({
    serverEvent: "pluginNotice",
    id: "0190f0a0-0000-7000-8000-0000000009f0",
    plugin: BOARDS,
    channel: roadmap,
    community,
    parentChannel: null,
    text: { key: "remind", args: { title: "Meetup" } },
    message: null,
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { shownNotices: unknown[] }).shownNotices),
    )
    .toEqual([{ title: "Boards", body: "Starts in an hour: Meetup" }]);
});
