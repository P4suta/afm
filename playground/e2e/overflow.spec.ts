import { expect, type Page, test } from '@playwright/test';

async function chooseOverflowAction(
  page: Page,
  name: string | RegExp,
): Promise<void> {
  await page.getByRole('button', { name: 'More' }).click();
  await page.getByRole('menuitem', { name }).click();
}

test.use({ viewport: { width: 900, height: 800 } });

test('reaches every primary action through the compact header menu', async ({
  context,
  page,
}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('./');
  await expect(page.locator('.playground-preview-host h1')).toBeVisible();
  await expect(page.getByRole('button', { name: 'More' })).toBeVisible();

  await chooseOverflowAction(page, 'Ruby and furigana');
  await expect(page.locator('.playground-preview-host h1')).toHaveText(
    'ルビ (振り仮名) いろいろ',
  );

  await chooseOverflowAction(page, 'Preview only');
  await expect(page.getByRole('region', { name: 'Editor' })).toBeHidden();
  await chooseOverflowAction(page, 'Split');
  await expect(page.getByRole('region', { name: 'Editor' })).toBeVisible();

  await chooseOverflowAction(page, 'Notation commands');
  await expect(
    page.getByRole('dialog', { name: 'Command palette' }),
  ).toBeVisible();
  await page.keyboard.press('Escape');

  await chooseOverflowAction(page, 'Guide');
  await expect(
    page.getByRole('dialog', { name: 'aozora-md notation guide' }),
  ).toBeVisible();
  await page.keyboard.press('Escape');

  await chooseOverflowAction(page, 'Share');
  await expect(page).toHaveURL(/#src=/);

  await chooseOverflowAction(page, 'Settings');
  await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
  await page.keyboard.press('Escape');

  await chooseOverflowAction(page, 'About this playground');
  await expect(
    page.getByRole('dialog', { name: 'About this playground' }),
  ).toBeVisible();
});
