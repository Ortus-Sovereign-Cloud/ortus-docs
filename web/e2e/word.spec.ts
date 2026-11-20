// Word and PDF: a document goes out as a .docx and comes back in as a new
// document with its headings, text and table; the PDF print window holds
// the document alone.
import { expect, test } from "@playwright/test";
import en from "../src/locales/en.js";

test("Word out and back in; the print window", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/docs");
  await page.locator("ortus-docs-home").getByRole("button", { name: en.newDocument }).click();
  const editor = page.locator("ortus-docs-editor");
  const text = editor.getByRole("textbox", { name: en.untitled });
  await text.click();
  await editor.getByRole("toolbar").getByLabel(en.style).selectOption("1");
  await page.keyboard.type("Evacuation routes");
  await page.keyboard.press("Enter");
  await editor.getByRole("toolbar").getByLabel(en.style).selectOption("p");
  await page.keyboard.type("Маршрути евакуації для району 4.");
  await page.keyboard.press("Enter");
  await editor.getByRole("toolbar").getByRole("button", { name: en.table }).click();
  await page.keyboard.type("District");
  await page.keyboard.press("Tab");
  await page.keyboard.type("Shelter");
  await expect(editor.getByText(en.saved)).toBeVisible();

  const [download] = await Promise.all([page.waitForEvent("download"), editor.getByRole("toolbar").getByRole("button", { name: en.downloadWord }).click()]);
  expect(download.suggestedFilename()).toBe(`${en.untitled}.docx`);
  const file = test.info().outputPath("routes.docx");
  await download.saveAs(file);

  const [popup] = await Promise.all([page.waitForEvent("popup"), editor.getByRole("toolbar").getByRole("button", { name: en.pdf }).click()]);
  await expect(popup.locator("main h1")).toHaveText("Evacuation routes");
  await expect(popup.locator("main table")).toContainText("Shelter");
  await expect(popup.locator(".ProseMirror-yjs-cursor")).toBeHidden();
  await popup.close();

  await page.goto("/docs");
  await page.locator("ortus-docs-home").getByLabel(en.importWord).setInputFiles(file);
  const imported = page.locator("ortus-docs-editor");
  await expect(imported.getByLabel(en.docName)).toHaveValue("routes");
  await expect(imported.locator(".ProseMirror h1")).toHaveText("Evacuation routes");
  await expect(imported.locator(".ProseMirror p").first()).toHaveText("Маршрути евакуації для району 4.");
  await expect(imported.locator(".ProseMirror table")).toContainText("District");
  await expect(imported.locator(".ProseMirror table")).toContainText("Shelter");
  // Kept: opened again, the imported text is there.
  await page.reload();
  await expect(page.locator("ortus-docs-editor").locator(".ProseMirror h1")).toHaveText("Evacuation routes");
  expect(errors).toEqual([]);
});
