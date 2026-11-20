// The editor inside another screen (a call's stage): it opens the
// document it is given, with no way back and no sharing or approval.
import { expect, test } from "@playwright/test";
import en from "../src/locales/en.js";

test("the editor opens a given document, embedded", async ({ page }) => {
  await page.goto("/docs");
  await page.locator("ortus-docs-home").getByRole("button", { name: en.newDocument }).click();
  const full = page.locator("ortus-docs-editor");
  await expect(full.getByRole("textbox", { name: en.untitled })).toBeVisible();
  await full.getByLabel(en.docName).fill("Shelter capacity");
  await full.getByLabel(en.docName).press("Enter");
  await expect(full.getByText(en.saved)).toBeVisible();
  const id = new URL(page.url()).searchParams.get("doc")!;

  await page.goto("/docs");
  await page.evaluate((doc) => {
    const el = document.createElement("ortus-docs-editor");
    el.setAttribute("doc", doc);
    el.setAttribute("embedded", "");
    el.id = "stage";
    document.body.append(el);
  }, id);
  const stage = page.locator("#stage");
  await expect(stage.getByLabel(en.docName)).toHaveValue("Shelter capacity");
  await expect(stage.getByRole("button", { name: en.back })).toHaveCount(0);
  await expect(stage.getByRole("button", { name: en.share })).toHaveCount(0);
  await expect(stage.getByRole("textbox", { name: "Shelter capacity" })).toBeVisible();
});
