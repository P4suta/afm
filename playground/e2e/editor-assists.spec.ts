import { expect, type Page, test } from '@playwright/test';

const clientErrors = new WeakMap<Page, string[]>();

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  clientErrors.set(page, errors);
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });

  await page.goto('./');
  await expect(
    page.getByRole('heading', {
      name: 'Aozora Flavored Markdown',
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator('.cm-editor')).toBeVisible();
  await expect(page.locator('.playground-preview-host h1')).toBeVisible();
});

test.afterEach(async ({ page }) => {
  expect(clientErrors.get(page) ?? []).toEqual([]);
});

async function replaceEditor(page: Page, source: string): Promise<void> {
  const editor = page.locator('.cm-content');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(source);
}

test.describe('real editor assistance', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('keeps Markdown heading Enter behavior free of Aozora snippets', async ({
    page,
  }) => {
    await replaceEditor(page, '');
    await page.keyboard.type('#');
    await page.waitForTimeout(200);
    await expect(page.locator('.cm-tooltip-autocomplete')).toHaveCount(0);
    await page.keyboard.type(' Heading');
    await page.keyboard.press('Enter');
    await page.keyboard.type('Paragraph');

    await expect(page.locator('.cm-content .cm-line')).toHaveText([
      '# Heading',
      'Paragraph',
    ]);
  });

  test('keeps GFM table-row Enter behavior free of ruby snippets', async ({
    page,
  }) => {
    await replaceEditor(page, '');
    await page.keyboard.type('| Name | Value |');
    await page.waitForTimeout(200);
    await expect(page.locator('.cm-tooltip-autocomplete')).toHaveCount(0);
    await page.keyboard.press('Enter');
    await page.keyboard.type('| --- | --- |');

    await expect(page.locator('.cm-content .cm-line')).toHaveText([
      '| Name | Value |',
      '| --- | --- |',
    ]);
  });

  test('localizes Japanese-only slug documentation in English', async ({
    page,
  }) => {
    await replaceEditor(page, '［＃');

    const details = page.locator(
      '.cm-tooltip-autocomplete .cm-completionDetail',
    );
    await expect(details.first()).toHaveText('Aozora notation');
    await expect(page.locator('.cm-tooltip-autocomplete')).not.toContainText(
      '見出し',
    );
  });

  test('completes an explicit ruby snippet and renders it through WASM', async ({
    page,
  }) => {
    await replaceEditor(page, '｜');
    const completion = page.getByRole('option', {
      name: /Ruby \(explicit\)/,
    });
    await expect(completion).toBeVisible();

    await completion.click();
    const editor = page.locator('.cm-content');
    await expect(editor).toContainText('｜base《reading》');
    await page.keyboard.insertText('漢字');
    await page.keyboard.press('Tab');
    await page.keyboard.insertText('かんじ');

    await expect(editor).toContainText('｜漢字《かんじ》');
    const ruby = page.locator('.playground-preview-host ruby');
    await expect(ruby).toContainText('漢字');
    await expect(ruby.locator('rt')).toHaveText('かんじ');
  });

  test('folds and unfolds a paired container from the real parser state', async ({
    page,
  }) => {
    await replaceEditor(
      page,
      '［＃ここから字下げ］\n折り畳まれる本文\n［＃ここで字下げ終わり］',
    );
    const editor = page.locator('.cm-content');
    await editor.press('Control+Home');
    await editor.press('Control+Shift+BracketLeft');

    await expect(page.locator('.cm-foldPlaceholder')).toBeVisible();
    await expect(editor).not.toContainText('折り畳まれる本文');

    await editor.press('Control+Shift+BracketRight');
    await expect(page.locator('.cm-foldPlaceholder')).toHaveCount(0);
    await expect(editor).toContainText('折り畳まれる本文');
  });

  test('preserves history, folds, and selection across a locale change', async ({
    page,
  }) => {
    const editor = page.locator('.cm-content');
    const editorShell = page.locator('.cm-editor');
    await replaceEditor(
      page,
      [
        'alpha',
        '［＃ここから字下げ］',
        'folded body',
        '［＃ここで字下げ終わり］',
        'omega',
      ].join('\n'),
    );
    await page.waitForTimeout(600);
    await page.keyboard.insertText('!');
    await page.waitForTimeout(600);

    await editor.press('Control+Home');
    await editor.press('ArrowDown');
    await editor.press('Control+Shift+BracketLeft');
    await expect(page.locator('.cm-foldPlaceholder')).toBeVisible();
    await expect(editor).not.toContainText('folded body');

    await editor.press('Control+End');
    await editor.press('Shift+Home');
    expect(await page.evaluate(() => getSelection()?.toString())).toBe(
      'omega!',
    );
    await editorShell.evaluate((element) => {
      element.setAttribute('data-editor-identity', 'retained');
    });

    await page.getByRole('button', { name: 'Settings' }).click();
    const settings = page.getByRole('dialog', { name: 'Settings' });
    await settings.getByRole('button', { name: /Language/ }).click();
    await page.getByRole('option', { name: 'Japanese' }).click();

    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
    await expect(editor).toHaveAttribute('aria-label', 'Markdown ソース');
    await expect(editorShell).toHaveAttribute(
      'data-editor-identity',
      'retained',
    );
    await page
      .getByRole('dialog', { name: '設定' })
      .getByRole('button', { name: '閉じる' })
      .click();
    await expect(page.getByRole('dialog', { name: '設定' })).toBeHidden();

    await editor.focus();
    await expect(page.locator('.cm-foldPlaceholder')).toBeVisible();
    await expect(editor).not.toContainText('folded body');
    expect(await page.evaluate(() => getSelection()?.toString())).toBe(
      'omega!',
    );

    await page.keyboard.press('ControlOrMeta+Z');
    await expect(editor).toContainText('omega');
    await expect(editor).not.toContainText('omega!');
    await expect(page.locator('.cm-foldPlaceholder')).toBeVisible();
  });

  test('renders and toggles parser-backed highlights and gaiji hints', async ({
    page,
  }) => {
    await replaceEditor(page, '｜漢字《かんじ》と外字 ※［＃二の字点、1-2-22］');
    const rubyHighlight = page.locator('.cm-aozora-ruby').first();
    await expect(rubyHighlight).toBeVisible();
    expect(
      Number.parseInt(
        await rubyHighlight.evaluate((node) => {
          return getComputedStyle(node).fontWeight;
        }),
        10,
      ),
    ).toBeGreaterThanOrEqual(600);

    const inlay = page.locator('.cm-aozora-inlay');
    await expect(inlay).toBeVisible();
    await expect(inlay).toContainText('→');

    await page.getByRole('button', { name: 'Settings' }).click();
    const highlighting = page.getByRole('switch', {
      name: 'Structure highlighting',
    });
    await highlighting.focus();
    await page.keyboard.press('Space');
    await expect(rubyHighlight).toHaveCount(0);
    await expect(inlay).toBeVisible();
  });
});
