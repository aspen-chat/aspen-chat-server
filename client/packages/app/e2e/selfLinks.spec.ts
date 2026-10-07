import { expect, test, type Page } from "@playwright/test";
import { bob, community, dm, general, longPress, signInToWorld, type Publish } from "./world";
import { messageId } from "./world/fixtures";

const bobName = "Bob With A Rather Long Display Name";
const postedId = "0190f0a0-0000-7000-8001-000000000981";

/** Opens #general, and has Bob post links to places of this deployment and one elsewhere. */
async function postLinks(
  page: Page,
  publish: Publish,
): Promise<{ channel: string; message: string; dm: string }> {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "general", exact: true })).toBeVisible();
  const origin = new URL(page.url()).origin;
  const links = {
    channel: `${origin}/communities/${community}/channels/${general}`,
    message: `${origin}/communities/${community}/channels/${general}/messages/${messageId(204)}`,
    dm: `${origin}/dms/${dm}`,
  };
  publish({
    serverEvent: "message",
    type: "create",
    id: postedId,
    channelId: general,
    author: bob,
    content: `Try ${links.channel}, ${links.message}, or ${links.dm}, or [my bank](https://bank.example/login)`,
    timestamp: new Date().toISOString(),
    editedAt: null,
    linkPreviews: [],
    linkedMessages: [messageId(204)],
    alteredBy: [],
    kind: "standard",
    poll: null,
    thread: null,
    echoOf: null,
    attachments: [],
    mentions: { users: [], roles: [], everyone: false },
  });
  return links;
}

/**
 * A chip's accessible name: its names with a comma after each but the last, which the
 * accessible name computation may pad with a space where the comma is visually hidden.
 */
const path = (...names: string[]) => new RegExp(`^${names.join("\\s*,\\s*")}$`);

const posted = (page: Page) => page.locator(`article[data-message-id="${postedId}"]`);

test("links to this deployment show the names of what they lead to", async ({ page }) => {
  const publish = await signInToWorld(page);
  const links = await postLinks(page, publish);
  const article = posted(page);
  const channel = article.getByRole("link", { name: path("Family", "general") });
  await expect(channel).toHaveAttribute("title", links.channel);
  await expect(
    article.getByRole("link", { name: path("Family", "general", `Message from ${bobName}`) }),
  ).toBeVisible();
  await expect(article.getByRole("link", { name: bobName, exact: true })).toBeVisible();
  // A link named by its author's words shows as written, its address linked on its own.
  await expect(article).toContainText("[my bank](https://bank.example/login)");
  await expect(article.getByRole("link", { name: "https://bank.example/login" })).toHaveAttribute(
    "target",
    "_blank",
  );
  await expect(article.getByRole("link", { name: "my bank" })).toHaveCount(0);
});

test("a link to this deployment opens in place, without loading the page again", async ({
  page,
}) => {
  const publish = await signInToWorld(page);
  await postLinks(page, publish);
  await page.evaluate(() => {
    (window as unknown as { stillHere?: boolean }).stillHere = true;
  });
  await posted(page).getByRole("link", { name: bobName, exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/dms/${dm}$`));
  expect(await page.evaluate(() => (window as unknown as { stillHere?: boolean }).stillHere)).toBe(
    true,
  );
});

test("right-clicking a link to this deployment offers its address", async ({ page, isMobile }) => {
  test.skip(isMobile, "a phone has no right click; the long press test covers it");
  const publish = await signInToWorld(page);
  const links = await postLinks(page, publish);
  await posted(page)
    .getByRole("link", { name: path("Family", "general") })
    .click({
      button: "right",
    });
  const menu = page.getByRole("menu", { name: "Link options" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Open original link" })).toHaveAttribute(
    "href",
    links.channel,
  );
  await menu.getByRole("menuitem", { name: "Copy link" }).click();
  await expect(page.getByText("Copied link", { exact: true })).toBeVisible();
});

test("holding a finger on a link to this deployment offers its address, not the message's actions", async ({
  page,
  isMobile,
}) => {
  test.skip(!isMobile, "a long press is a touch screen's");
  const publish = await signInToWorld(page);
  const links = await postLinks(page, publish);
  await longPress(page, posted(page).getByRole("link", { name: path("Family", "general") }));
  const menu = page.getByRole("menu", { name: "Link options" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Open original link" })).toHaveAttribute(
    "href",
    links.channel,
  );
  await expect(page.getByRole("dialog", { name: "Message actions" })).toHaveCount(0);
  // Still on the channel: the press opened the menu rather than the link.
  await expect(page).toHaveURL(new RegExp(`/channels/${general}$`));
});
