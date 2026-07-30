import { expect, test } from '@playwright/test';

test('keeps renderer-looking source inert at the innerHTML boundary', async ({
  page,
}) => {
  await page.goto('./');
  const editor = page.locator('.cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(
    '<img src=x onerror="globalThis.__afmInjected = true">',
  );

  const preview = page.locator('.playground-preview-host');
  await expect(preview.locator('.aozora-md-root')).toBeEmpty();
  await expect(preview.locator('img')).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        (
          globalThis as typeof globalThis & {
            __afmInjected?: boolean;
          }
        ).__afmInjected,
    ),
  ).toBeUndefined();
});
