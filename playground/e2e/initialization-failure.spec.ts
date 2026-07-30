import { expect, test } from '@playwright/test';

test('keeps the editor usable and retries after the first WASM request fails', async ({
  page,
}) => {
  let failed = false;
  await page.route(/\.wasm(?:$|\?)/, async (route) => {
    if (!failed) {
      failed = true;
      await route.abort();
    } else {
      await route.continue();
    }
  });

  await page.goto('./');
  const editor = page.locator('.cm-content');
  await expect(editor).toBeVisible();
  await expect(
    page.getByText('WebAssembly failed to initialize.'),
  ).toBeVisible();
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText('# Draft survives retry');

  await page.getByRole('button', { name: 'Retry' }).click();
  await expect(page.locator('.playground-preview-host h1')).toHaveText(
    'Draft survives retry',
  );
});
