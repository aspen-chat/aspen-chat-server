import { expect, test, type Page } from "@playwright/test";
import {
  bob,
  community,
  dm,
  general,
  organiserRoleWith,
  ownText,
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
