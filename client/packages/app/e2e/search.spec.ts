import { expect, test, type Page } from "@playwright/test";
import { foreignDomain, foreignSearchText, stubForeignDeployment } from "./foreignWorld";
import {
  community,
  deepSearchPosition,
  deepSearchText,
  general,
  ownText,
  signInToWorld,
  thread,
} from "./world";

/**
 * Message search against the stubbed world: it finds messages by their words, says where each
 * was said, opens it in place, and searches every server the user uses at once.
 */

async function openSearch(page: Page) {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await page.getByRole("button", { name: "Search messages" }).click();
  return page.getByRole("dialog", { name: "Search messages" });
}

test("a search finds messages by their words and opens one in place", async ({ page }) => {
  await signInToWorld(page);
  const dialog = await openSearch(page);
  const asked = page.waitForRequest((request) =>
    new URL(request.url()).pathname.endsWith("/messages"),
  );
  await dialog.getByRole("searchbox", { name: "Words" }).fill("LEMONADE chairs");
  await dialog.getByRole("button", { name: "Search", exact: true }).click();
  const query = new URL((await asked).url()).searchParams;
  expect(query.get("filter[text]")).toBe("LEMONADE chairs");
  expect(query.get("filter[community]")).toBe(community);
  const results = dialog.getByRole("region", { name: "Results" });
  const found = results.getByRole("link", { name: /Go to message: Kate/ });
  await expect(found).toContainText(ownText);
  await expect(found).toContainText("#general in Family");
  await found.click();
  await expect(dialog).toBeHidden();
  await expect(page).toHaveURL(new RegExp(`/channels/${general}/messages/`));
});

test("a search that finds nothing near the present is asked again from where it read to", async ({
  page,
}) => {
  await signInToWorld(page);
  const dialog = await openSearch(page);
  const befores: (string | null)[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.endsWith("/messages") && url.searchParams.has("filter[text]")) {
      befores.push(url.searchParams.get("before"));
    }
  });
  await dialog.getByRole("searchbox", { name: "Words" }).fill(deepSearchText);
  await dialog.getByRole("button", { name: "Search", exact: true }).click();
  const results = dialog.getByRole("region", { name: "Results" });
  await expect(results.getByRole("link", { name: /Go to message: Kate/ })).toContainText(ownText);
  expect(befores).toEqual([null, deepSearchPosition]);
});

test("a reply found in a thread opens its thread", async ({ page }) => {
  await signInToWorld(page);
  const dialog = await openSearch(page);
  await dialog.getByRole("radio", { name: "#general" }).click();
  const asked = page.waitForRequest((request) =>
    new URL(request.url()).pathname.endsWith("/messages"),
  );
  await dialog.getByRole("searchbox", { name: "Words" }).fill("crisps");
  await dialog.getByRole("button", { name: "Search", exact: true }).click();
  expect(new URL((await asked).url()).searchParams.get("filter[channel]")).toBe(general);
  const found = dialog.getByRole("link", { name: /Go to message: Bob/ });
  await expect(found).toContainText("A thread in #general in Family");
  await found.click();
  await expect(page).toHaveURL(new RegExp(`/threads/${thread}$`));
});

test("a search that finds nothing says so, and one with nothing to find cannot start", async ({
  page,
}) => {
  await signInToWorld(page);
  const dialog = await openSearch(page);
  const submit = dialog.getByRole("button", { name: "Search", exact: true });
  await expect(submit).toBeDisabled();
  await dialog.getByRole("searchbox", { name: "Words" }).fill("zeppelin");
  await submit.click();
  await expect(dialog.getByText("Nothing matches.")).toBeVisible();
});

test("all servers are searched at once, newest first", async ({ page }) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: true }));
  await expect(
    page
      .getByRole("navigation", { name: "Communities" })
      .getByRole("row", { name: `Beta club on ${foreignDomain}` }),
  ).toBeVisible();
  const dialog = await openSearch(page);
  await dialog.getByRole("radio", { name: "All servers" }).click();
  await dialog.getByRole("searchbox", { name: "Words" }).fill("lemonade");
  await dialog.getByRole("button", { name: "Search", exact: true }).click();
  const results = dialog.getByRole("region", { name: "Results" }).getByRole("link");
  await expect(results).toHaveCount(2);
  await expect(results.first()).toContainText(foreignSearchText);
  await expect(results.first()).toContainText(`on ${foreignDomain}`);
  await expect(results.nth(1)).toContainText(ownText);
});

test("a search by its author can be widened to anyone again", async ({ page }) => {
  await signInToWorld(page);
  const dialog = await openSearch(page);
  const from = dialog.getByRole("combobox", { name: "From" });
  await expect(from).toHaveAttribute("placeholder", "Anyone");
  await dialog.getByRole("button", { name: /^Show suggestions/ }).click();
  await page.getByRole("option", { name: /^Bob With A Rather Long Display Name/ }).click();
  const searched = () =>
    page.waitForRequest((request) => new URL(request.url()).pathname.endsWith("/messages"));
  let asked = searched();
  await dialog.getByRole("searchbox", { name: "Words" }).fill("crisps");
  await dialog.getByRole("button", { name: "Search", exact: true }).click();
  expect(new URL((await asked).url()).searchParams.get("filter[author]")).not.toBeNull();
  await dialog.getByRole("button", { name: /^Show suggestions/ }).click();
  await page.getByRole("option", { name: "Anyone", exact: true }).click();
  await expect(page.getByRole("listbox")).toBeHidden();
  await expect(from).toHaveValue("");
  asked = searched();
  await dialog.getByRole("button", { name: "Search", exact: true }).click();
  expect(new URL((await asked).url()).searchParams.get("filter[author]")).toBeNull();
});

test("emptying the author field stops narrowing the search by them", async ({ page }) => {
  await signInToWorld(page);
  const dialog = await openSearch(page);
  const from = dialog.getByRole("combobox", { name: "From" });
  await dialog.getByRole("button", { name: /^Show suggestions/ }).click();
  await page.getByRole("option", { name: /^Bob With A Rather Long Display Name/ }).click();
  await from.fill("");
  await from.press("Escape");
  const asked = page.waitForRequest((request) =>
    new URL(request.url()).pathname.endsWith("/messages"),
  );
  await dialog.getByRole("searchbox", { name: "Words" }).fill("crisps");
  await dialog.getByRole("button", { name: "Search", exact: true }).click();
  expect(new URL((await asked).url()).searchParams.get("filter[author]")).toBeNull();
});
