import { expect, type Page, test } from '@playwright/test';

function collectClientErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  return errors;
}

function unexpectedClientErrors(errors: readonly string[]): readonly string[] {
  return errors.filter(
    (message) =>
      !message.includes(
        'Invalid value for <circle> attribute r="calc(50% - 0.09375rem)"',
      ),
  );
}

async function replaceEditor(page: Page, source: string): Promise<void> {
  const editor = page.locator('.cm-content');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(source);
}

test('authors and renders with the real AFM engine', async ({ page }) => {
  const errors = collectClientErrors(page);
  await page.goto('./');

  const editor = page.locator('.cm-content');
  await expect(editor).toBeVisible({ timeout: 30_000 });
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText('明治33［＃「33」は縦中横］年');
  await page.getByRole('radio', { name: 'Vertical' }).click();

  const tateChuYoko = page.locator(
    '.playground-preview-host .aozora-md-combine-upright',
  );
  await expect(tateChuYoko).toContainText('33');
  await expect
    .poll(() =>
      tateChuYoko.evaluate(
        (element) => getComputedStyle(element).textCombineUpright,
      ),
    )
    .toBe('all');
  expect(unexpectedClientErrors(errors)).toEqual([]);
});

test('opens the keyboard palette and restores editor focus after a command', async ({
  page,
}) => {
  const errors = collectClientErrors(page);
  await page.goto('./');
  await expect(page.locator('.cm-content')).toBeVisible({ timeout: 30_000 });
  await replaceEditor(page, '漢字');
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.press('ControlOrMeta+Shift+P');
  await expect(
    page.getByRole('dialog', { name: 'Command palette' }),
  ).toBeVisible();
  await page.getByRole('button', { name: /Ruby/ }).click();

  await expect(page.locator('.cm-content')).toBeFocused();
  await page.keyboard.insertText('かんじ');
  await expect(page.locator('.playground-preview-host ruby')).toContainText(
    '漢字',
  );
  expect(unexpectedClientErrors(errors)).toEqual([]);
});

test('persists a localized dark draft and restores dialog focus', async ({
  page,
}) => {
  const errors = collectClientErrors(page);
  await page.goto('./');
  await expect(page.locator('.cm-content')).toBeVisible({ timeout: 30_000 });
  await replaceEditor(page, '# Cross-browser draft');

  await page.getByRole('button', { name: 'Settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('button', { name: /Theme/ }).click();
  await page.getByRole('option', { name: 'Dark' }).click();
  await settings.getByRole('button', { name: /Language/ }).click();
  const japaneseOption = page.getByRole('option', { name: 'Japanese' });
  await japaneseOption.click();
  await expect(japaneseOption).toBeHidden();
  const languageListbox = page.getByRole('listbox');
  if (await languageListbox.isVisible()) {
    await page.keyboard.press('Escape');
    await expect(languageListbox).toBeHidden();
  }
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: '設定' })).toBeHidden();
  await expect(page.getByRole('button', { name: '設定' })).toBeFocused();

  await page.waitForTimeout(350);
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
  await expect(page.locator('html')).toHaveAttribute(
    'data-color-scheme',
    'dark',
  );
  await expect(page.locator('.cm-content')).toContainText(
    'Cross-browser draft',
  );
  await expect(page.locator('.playground-preview-host h1')).toHaveText(
    'Cross-browser draft',
  );
  expect(unexpectedClientErrors(errors)).toEqual([]);
});

test('keeps edits while retrying a failed initial WASM request', async ({
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
  await expect(page.locator('.cm-content')).toBeVisible({ timeout: 30_000 });
  await expect(
    page.getByText('WebAssembly failed to initialize.'),
  ).toBeVisible();
  await replaceEditor(page, '# Cross-browser retry');
  await page.getByRole('button', { name: 'Retry' }).click();
  await expect(page.locator('.playground-preview-host h1')).toHaveText(
    'Cross-browser retry',
  );
});

test('keeps one editor session across responsive layouts', async ({ page }) => {
  const errors = collectClientErrors(page);
  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto('./');
  const editor = page.locator('.cm-content');
  const editorShell = page.locator('.cm-editor');
  await expect(editor).toBeVisible({ timeout: 30_000 });
  await replaceEditor(page, 'cross-browser history marker');
  await editor.press('Shift+Home');
  const originalEditor = await editorShell.elementHandle();

  await page.getByRole('tab', { name: 'Preview' }).click();
  await expect(editorShell).toBeHidden();
  await page.getByRole('tab', { name: 'Editor' }).click();
  await expect(editorShell).toBeVisible();

  await page.setViewportSize({ width: 900, height: 800 });
  await expect(page.getByRole('tab', { name: 'Editor' })).toHaveCount(0);
  await page.setViewportSize({ width: 320, height: 720 });
  await expect(page.getByRole('tab', { name: 'Editor' })).toBeVisible();

  const returnedEditor = await editorShell.elementHandle();
  expect(
    await originalEditor?.evaluate(
      (original, returned) => original.isSameNode(returned),
      returnedEditor,
    ),
  ).toBe(true);
  await editor.focus();
  await expect
    .poll(() => page.evaluate(() => getSelection()?.toString() ?? ''))
    .toBe('cross-browser history marker');
  await page.keyboard.press('ControlOrMeta+Z');
  await expect(editor).not.toContainText('cross-browser history marker');
  expect(unexpectedClientErrors(errors)).toEqual([]);
});

test('keeps the 320px authoring page fixed and filled', async ({ page }) => {
  const errors = collectClientErrors(page);
  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto('./');

  await expect(page.locator('.cm-editor')).toBeVisible({ timeout: 30_000 });
  const editorBox = await page
    .getByRole('region', { name: 'Editor' })
    .boundingBox();
  expect(editorBox).not.toBeNull();
  expect((editorBox?.y ?? 0) + (editorBox?.height ?? 0)).toBeGreaterThanOrEqual(
    716,
  );
  const dimensions = await page.evaluate(() => ({
    documentHeight: document.documentElement.scrollHeight,
    viewportHeight: window.innerHeight,
  }));
  expect(dimensions.documentHeight).toBeLessThanOrEqual(
    dimensions.viewportHeight,
  );

  await page.getByRole('tab', { name: 'Preview' }).click();
  await expect(page.locator('.playground-preview-host h1')).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    ),
  ).toBeLessThanOrEqual(1);
  expect(unexpectedClientErrors(errors)).toEqual([]);
});
