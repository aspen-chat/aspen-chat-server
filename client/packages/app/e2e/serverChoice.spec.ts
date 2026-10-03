import { expect, test } from "@playwright/test";
import { stubAuthMethods } from "./stubs";

/**
 * Choosing a deployment in a shell with no server of its own. The desktop shell is stood in for
 * by its bridge, `window.aspenDesktop`, which is how the app tells it apart from the web.
 */
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    (window as { aspenDesktop?: unknown }).aspenDesktop = {};
  });
  await stubAuthMethods(page);
});

test("a first launch asks for a deployment, and keeps only its origin", async ({
  page,
  baseURL,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Welcome to Aspen Chat." })).toBeVisible();
  await expect(page.locator("main img")).toBeVisible();
  const field = page.getByRole("textbox", { name: "Deployment URL" });
  // No address is baked in.
  await expect(field).toHaveValue("");
  await field.fill("ftp://chat.example.org");
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByText("Enter a deployment URL such as chat.example.org.")).toBeVisible();
  const origin = new URL(baseURL ?? "").origin;
  await field.fill(`${origin}/communities/abc?at=elsewhere#top`);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByText(`Server: ${origin}`)).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem("aspen.serverUrl"))).toBe(origin);
});

test("changing the deployment starts from an empty field, with a way back", async ({
  page,
  baseURL,
}) => {
  const origin = new URL(baseURL ?? "").origin;
  await page.addInitScript((server) => {
    localStorage.setItem("aspen.serverUrl", server);
  }, origin);
  await page.goto("/");
  await page.getByRole("button", { name: "Change" }).click();
  await expect(page.getByRole("textbox", { name: "Deployment URL" })).toHaveValue("");
  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByText(`Server: ${origin}`)).toBeVisible();
});
