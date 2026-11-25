// The editor (board WebDocEditor) and review (board WebReview): writing
// with a heading and a table, a second person's change arriving live,
// a suggestion accepted, a comment, and the review route.
import { expect, test, type Page } from "@playwright/test";
import en from "../src/locales/en.js";

test.describe.configure({ mode: "serial" });

const editor = (page: Page) => page.locator("ortus-docs-editor");
let address = "";

test("a new document is written and saved", async ({ page }) => {
  await page.goto("/docs");
  await page.locator("ortus-docs-home").getByRole("button", { name: en.newDocument }).click();
  await expect(editor(page).getByRole("textbox", { name: en.untitled })).toBeVisible();
  address = page.url();
  // Focus moves to the new document's name, ready to type over.
  await expect(editor(page).getByLabel(en.docName)).toBeFocused();
  await editor(page).getByLabel(en.docName).fill("Flood response plan 2026");
  await editor(page).getByLabel(en.docName).press("Enter");
  const text = editor(page).getByRole("textbox", { name: en.untitled });
  await text.click();
  await editor(page).getByRole("toolbar").getByLabel(en.style).selectOption("1");
  await page.keyboard.type("Flood response plan 2026");
  await page.keyboard.press("Enter");
  await editor(page).getByRole("toolbar").getByLabel(en.style).selectOption("p");
  await page.keyboard.type("Each district opens at least one shelter. District 4 uses the sports hall.");
  await expect(editor(page).getByText(en.saved)).toBeVisible();
});

test("a broken update from someone does not stop the document opening", async ({ page }) => {
  const id = new URL(address).searchParams.get("doc");
  const r = await page.request.post(`/api/v1/docs/documents/${id}/updates?client=bad`, { headers: { "X-Requested-With": "ortus", "Content-Type": "application/octet-stream" }, data: Buffer.from([1, 2, 3]) });
  expect(r.ok()).toBeTruthy();
  await page.goto(address);
  await expect(editor(page).locator(".ProseMirror")).toContainText("sports hall");
});

test("a change made elsewhere arrives live; a suggestion is accepted", async ({ page, browser }) => {
  await page.goto(address);
  await expect(editor(page).locator(".ProseMirror")).toContainText("sports hall");
  // Another window (the same person, as another editor would be).
  const other = await (await browser.newContext({ storageState: "test-results/owner.json" })).newPage();
  await other.goto(address);
  const there = other.locator("ortus-docs-editor");
  const errors: string[] = [];
  other.on("pageerror", (e) => errors.push(e.message));
  other.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await expect(there.locator(".ProseMirror")).toContainText("sports hall");
  await there.getByLabel(en.mode).selectOption("suggesting");
  const p = there.locator(".ProseMirror p").first();
  await p.click();
  await other.keyboard.press("End");
  await other.keyboard.type(" Buses leave from the central depot.");
  await expect(editor(page).locator(".ProseMirror ins")).toContainText("Buses leave", { timeout: 10_000 });
  const card = editor(page).getByRole("region", { name: /Suggestion by/ });
  await expect(card).toContainText("Buses leave from the central depot.");
  await card.getByRole("button", { name: en.accept }).click();
  await expect(editor(page).locator(".ProseMirror ins")).toHaveCount(0);
  await expect(there.locator(".ProseMirror")).toContainText("Buses leave from the central depot.");
  await expect(there.locator(".ProseMirror ins")).toHaveCount(0, { timeout: 10_000 });
  expect(errors).toEqual([]);
  await other.context().close();
});

test("a comment on a passage", async ({ page }) => {
  await page.goto(address);
  const text = editor(page).locator(".ProseMirror");
  await expect(text).toContainText("shelter");
  await text.getByText("sports hall", { exact: false }).dblclick();
  await editor(page).getByRole("toolbar").getByRole("button", { name: en.comment }).click();
  const form = editor(page).getByRole("form", { name: en.newComment });
  await form.getByRole("textbox", { name: en.comment }).fill("Use the school on Park Street instead.");
  await form.getByRole("button", { name: en.comment }).click();
  await expect(editor(page).getByRole("region", { name: /Comment by/ })).toContainText("school on Park Street");
  await expect(text.locator("mark.comment")).toHaveCount(1);
  await page.screenshot({ path: "test-results/editor.png" });
});

test("sent for approval, reviewed, approved", async ({ page }) => {
  await page.goto(address);
  await expect(editor(page).locator(".ProseMirror")).toContainText("shelter");
  await editor(page).getByRole("button", { name: en.sendForApproval }).click();
  const dialog = editor(page).getByRole("dialog", { name: en.sendForApproval });
  await dialog.getByLabel("Reviewer 1", { exact: true }).fill("olena@ministry.example");
  await dialog.getByLabel("What reviewer 1 does").fill("Logistics review");
  await dialog.getByRole("button", { name: en.send }).click();
  await expect(dialog).toBeHidden();
  await expect(editor(page).getByText(en.inReview)).toBeVisible();
  await page.goto("/docs");
  await page.locator("ortus-docs-home").getByRole("link", { name: /Flood response plan 2026/ }).click();
  const review = page.locator("ortus-docs-review");
  await expect(review.getByRole("article")).toContainText("Each district opens at least one shelter.");
  await expect(review.getByRole("region", { name: en.yourTurn })).toContainText("Logistics review");
  await page.screenshot({ path: "test-results/review.png" });
  await review.getByRole("button", { name: en.approve }).click();
  await expect(review.getByRole("status")).toHaveText(en.allApproved);
});
