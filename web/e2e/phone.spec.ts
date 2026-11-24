// On a phone (390 x 844): nothing wider than the screen, one pane at a
// time. The editor's page fills the width and its toolbar wraps; the
// comments are behind a button with a way back; the review shows the
// document first and the decision behind a button with a way back.
import { expect, test, type Locator, type Page } from "@playwright/test";
import en from "../src/locales/en.js";

test.use({ viewport: { width: 390, height: 844 } });

async function fits(page: Page, app?: Locator): Promise<void> {
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  if (app) {
    const [scroll, client] = await app.evaluate((el) => [el.scrollWidth, el.clientWidth]);
    expect(scroll).toBeLessThanOrEqual(client);
  }
}

/** The element takes (nearly) the app's whole width. */
async function fullWidth(app: Locator, part: Locator): Promise<void> {
  const outer = await app.boundingBox();
  const inner = await part.boundingBox();
  expect(inner && outer && inner.width >= outer.width - 32).toBe(true);
}

test("docs on a phone: the list, the editor with its comments, the review", async ({ page }) => {
  await page.goto("/docs");
  const home = page.locator("ortus-docs-home");
  await expect(home.getByRole("button", { name: en.newDocument })).toBeVisible();
  await fits(page, home);
  await page.screenshot({ path: "test-results/phone-docs-home.png" });

  // The editor: the page and the toolbar fit.
  await home.getByRole("button", { name: en.newDocument }).click();
  const editor = page.locator("ortus-docs-editor");
  await editor.getByLabel(en.docName).fill("Evacuation routes for the river districts");
  await editor.getByLabel(en.docName).press("Enter");
  const text = editor.locator(".ProseMirror");
  await text.click();
  await page.keyboard.type("Route A runs along the ring road to the stadium car park.");
  await page.keyboard.press("Enter");
  // A table: it scrolls inside its own box if it must.
  await editor.getByRole("toolbar").getByRole("button", { name: en.table, exact: true }).click();
  for (const cell of ["Route", "Starts at", "Ends at", "A", "Riverside school", "Stadium car park", "B", "Old town square", "Hill district clinic"]) {
    await page.keyboard.type(cell);
    await page.keyboard.press("Tab");
  }
  await expect(text.locator("table")).toContainText("Hill district clinic");
  await expect(editor.getByText(en.saved)).toBeVisible();
  await fullWidth(editor, editor.locator(".page"));
  await fits(page, editor);
  await page.screenshot({ path: "test-results/phone-docs-editor.png" });

  // A comment: the notes are the whole screen, then back to the page.
  await text.getByText("ring road", { exact: false }).dblclick();
  await editor.getByRole("toolbar").getByRole("button", { name: en.comment, exact: true }).click();
  const form = editor.getByRole("form", { name: en.newComment });
  await expect(editor.locator(".page")).toBeHidden();
  await form.getByRole("textbox", { name: en.comment }).fill("Add the second car park.");
  await form.getByRole("button", { name: en.comment }).click();
  await expect(editor.getByRole("region", { name: /Comment by/ })).toContainText("second car park");
  await fullWidth(editor, editor.getByRole("complementary", { name: en.notes }));
  await fits(page, editor);
  await page.screenshot({ path: "test-results/phone-docs-notes.png" });
  await editor.getByRole("complementary", { name: en.notes }).getByRole("button", { name: en.back, exact: true }).click();
  await expect(editor.locator(".page")).toBeVisible();
  await expect(editor.getByRole("region", { name: /Comment by/ })).toBeHidden();
  await editor.getByRole("toolbar").getByRole("button", { name: /Comments and suggestions \(1\)/ }).click();
  await expect(editor.getByRole("region", { name: /Comment by/ })).toBeVisible();
  await editor.getByRole("complementary", { name: en.notes }).getByRole("button", { name: en.back, exact: true }).click();

  // Sent for approval: the dialog fits.
  await editor.getByRole("button", { name: en.sendForApproval }).click();
  const dialog = editor.getByRole("dialog", { name: en.sendForApproval });
  await dialog.getByLabel("Reviewer 1", { exact: true }).fill("olena@ministry.example");
  await dialog.getByLabel("What reviewer 1 does").fill("Route check");
  expect((await dialog.boundingBox())!.width).toBeLessThanOrEqual(390);
  await page.screenshot({ path: "test-results/phone-docs-approval.png" });
  await dialog.getByRole("button", { name: en.send }).click();
  await expect(dialog).toBeHidden();

  // The review: the document first, the decision behind a button.
  await page.goto("/docs");
  await home.getByRole("link", { name: /Evacuation routes/ }).click();
  const review = page.locator("ortus-docs-review");
  const article = review.getByRole("article");
  await expect(article).toContainText("ring road");
  await expect(article.locator("table")).toContainText("Hill district clinic");
  await fullWidth(review, article);
  await fits(page, review);
  await page.screenshot({ path: "test-results/phone-docs-review.png" });
  await review.getByRole("button", { name: en.yourTurn, exact: true }).click();
  const turn = review.getByRole("region", { name: en.yourTurn });
  await expect(turn).toContainText("Route check");
  await expect(article).toBeHidden();
  await fullWidth(review, turn);
  await fits(page, review);
  await page.screenshot({ path: "test-results/phone-docs-decision.png" });
  await review.locator("aside").getByRole("button", { name: en.back, exact: true }).click();
  await expect(article).toBeVisible();
  await review.getByRole("button", { name: en.yourTurn, exact: true }).click();
  await review.getByRole("button", { name: en.approve }).click();
  await expect(review.getByRole("status")).toHaveText(en.allApproved);
  // Back to the list.
  await review.locator("header").getByRole("button", { name: en.back, exact: true }).click();
  await expect(home.getByRole("button", { name: en.newDocument })).toBeVisible();
});
