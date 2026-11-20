// A route that signs (board WebApprovals): steps that sign or approve, a
// case; the author signs, then the route is done and its record linked.
import { expect, test } from "@playwright/test";
import en from "../src/locales/en.js";

test("a document is signed in a route and filed", async ({ page }) => {
  await page.goto("/docs");
  await page.locator("ortus-docs-home").getByRole("button", { name: en.newDocument }).click();
  const editor = page.locator("ortus-docs-editor");
  await editor.getByLabel(en.docName).fill("Shelter rota");
  await editor.getByLabel(en.docName).press("Enter");
  await editor.getByRole("textbox", { name: en.untitled }).click();
  await page.keyboard.type("Approved for use by all districts.");
  await expect(editor.getByText(en.saved)).toBeVisible();
  await editor.getByRole("button", { name: en.sendForApproval }).click();
  const send = editor.getByRole("dialog", { name: en.sendForApproval });
  await send.getByLabel("Reviewer 1", { exact: true }).fill("olena@ministry.example");
  await send.getByLabel("What reviewer 1 does", { exact: true }).fill("Author signs");
  await send.getByLabel("Reviewer 1 approves or signs").selectOption("sign");
  await send.getByLabel(en.caseRef).fill("2026/EM/0114");
  await send.getByRole("button", { name: en.send, exact: true }).click();
  await expect(send).toBeHidden();

  await page.goto("/docs");
  await page.locator("ortus-docs-home").getByRole("link", { name: /Shelter rota/ }).click();
  const review = page.locator("ortus-docs-review");
  await expect(review.locator(".case")).toContainText("2026/EM/0114");
  await expect(review.getByRole("region", { name: en.route })).toContainText(en.fileStep);
  await expect(review.locator(".box")).toContainText(["Author signs"]);
  await page.screenshot({ path: "test-results/sign.png" });
  await review.getByRole("button", { name: en.sign }).click();
  await expect(review.getByRole("status")).toContainText(en.allSigned);
  await expect(review.getByRole("link", { name: en.openRecord })).toHaveAttribute("href", "/records?record=r1");
  await expect(review.locator(".box.done")).toContainText("Signed");
});
