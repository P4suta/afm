import AxeBuilder from '@axe-core/playwright';
import { expect, type Page, test } from '@playwright/test';
import LZString from 'lz-string';

const clientErrors = new WeakMap<Page, string[]>();

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  clientErrors.set(page, errors);
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
});

test.afterEach(async ({ page }) => {
  expect(clientErrors.get(page) ?? []).toEqual([]);
});

async function openPlayground(page: Page): Promise<void> {
  await page.goto('./');
  await expect(
    page.getByRole('heading', {
      name: 'Aozora Flavored Markdown',
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator('.cm-editor')).toBeVisible();
  if ((page.viewportSize()?.width ?? 0) >= 768) {
    await expect(page.locator('.playground-preview-host h1')).toBeVisible();
  }
}

async function replaceEditor(page: Page, source: string): Promise<void> {
  const editor = page.locator('.cm-content');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(source);
}

async function expectNoAxeViolations(page: Page): Promise<void> {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(results.violations).toEqual([]);
}

test.describe('deferred engine startup', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('keeps the basic editor usable while the production WASM request is pending', async ({
    page,
  }) => {
    let releaseWasmRequest: (() => void) | undefined;
    const wasmRequestReleased = new Promise<void>((resolve) => {
      releaseWasmRequest = resolve;
    });
    let observeWasmRequest: (() => void) | undefined;
    const wasmRequestObserved = new Promise<void>((resolve) => {
      observeWasmRequest = resolve;
    });
    await page.route(/\.wasm(?:$|\?)/, async (route) => {
      observeWasmRequest?.();
      await wasmRequestReleased;
      await route.continue();
    });

    await page.goto('./');
    await wasmRequestObserved;
    await expect(
      page.getByRole('heading', {
        name: 'Aozora Flavored Markdown',
        exact: true,
      }),
    ).toBeVisible();
    const editor = page.locator('.cm-content');
    await expect(editor).toBeVisible();
    await editor.click();
    await page.keyboard.press('ControlOrMeta+A');
    await page.keyboard.insertText('# Edited before engine readiness');
    await expect(editor).toContainText('Edited before engine readiness');
    await expect(page.locator('.playground-preview-host h1')).toHaveCount(0);

    releaseWasmRequest?.();
    await expect(page.locator('.playground-preview-host h1')).toHaveText(
      'Edited before engine readiness',
    );
  });

  test('keeps the WASM failure alert accessible', async ({ page }) => {
    await page.route(/\.wasm(?:$|\?)/, async (route) => {
      await route.fulfill({
        body: Buffer.from([0x00, 0x61, 0x73, 0x6d]),
        contentType: 'application/wasm',
        status: 200,
      });
    });

    await page.goto('./');
    await expect(page.locator('.cm-editor')).toBeVisible();
    const alert = page.getByRole('alert');
    await expect(alert).toContainText('WebAssembly failed to initialize.');
    await expect(page.getByRole('button', { name: 'Retry' })).toBeVisible();
    await expectNoAxeViolations(page);
  });
});

test.describe('desktop authoring workspace', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await openPlayground(page);
  });

  test('renders the authoring frame, command palette, and fixed pane scrolling', async ({
    page,
  }) => {
    await page.keyboard.press('Control+Shift+P');
    await expect(
      page.getByRole('dialog', { name: 'Command palette' }),
    ).toBeVisible();
    await expectNoAxeViolations(page);

    const dimensions = await page.evaluate(() => ({
      body: document.body.scrollHeight,
      document: document.documentElement.scrollHeight,
      viewport: window.innerHeight,
    }));
    expect(dimensions.body).toBeLessThanOrEqual(dimensions.viewport);
    expect(dimensions.document).toBeLessThanOrEqual(dimensions.viewport);
  });

  test('switches all desktop layouts and opens the persistent outline', async ({
    page,
  }) => {
    await page.getByRole('radio', { name: 'Editor only' }).click();
    await expect(page.getByRole('region', { name: 'Preview' })).toBeHidden();
    await page.getByRole('radio', { name: 'Preview only' }).click();
    await expect(page.getByRole('region', { name: 'Editor' })).toBeHidden();
    await page.getByRole('radio', { name: 'Split' }).click();

    await page.getByRole('button', { name: 'Outline' }).click();
    const outline = page.getByRole('complementary', { name: 'Outline' });
    await expect(outline).toBeVisible();
    await expect(outline.getByRole('listitem').first()).toHaveAttribute(
      'aria-level',
      '1',
    );
    await outline
      .getByRole('button', {
        name: 'aozora-md へようこそ, heading level 1',
      })
      .click();
    await expect(page.locator('.cm-content')).toBeFocused();
  });

  test('runs a notation command against the CodeMirror selection and updates real WASM output', async ({
    page,
  }) => {
    await replaceEditor(page, '漢字');
    await page.keyboard.press('ControlOrMeta+A');
    await page.keyboard.press('Control+Shift+P');
    await page.getByRole('button', { name: /Ruby/ }).click();
    await expect(page.locator('.cm-content')).toBeFocused();
    await page.keyboard.insertText('かんじ');

    await expect(page.locator('.cm-content')).toContainText('｜漢字《かんじ》');
    await expect(page.locator('.playground-preview-host ruby')).toContainText(
      '漢字',
    );
  });

  test('preserves selection and undo history through preview-only commands', async ({
    page,
  }) => {
    const editor = page.locator('.cm-content');
    await replaceEditor(page, 'alpha beta');
    await page.waitForTimeout(600);
    await editor.click();
    await page.keyboard.press('Home');
    await page.keyboard.insertText('X');
    await page.waitForTimeout(600);
    await page.keyboard.press('End');
    for (let index = 0; index < 4; index++) {
      await page.keyboard.press('Shift+ArrowLeft');
    }
    expect(await page.evaluate(() => getSelection()?.toString())).toBe('beta');

    await page.getByRole('radio', { name: 'Preview only' }).click();
    await page.keyboard.press('Control+Shift+P');
    await page.getByRole('button', { name: /Ruby/ }).click();

    await expect(page.getByRole('radio', { name: 'Split' })).toBeChecked();
    await expect(editor).toBeFocused();
    await expect(editor).toContainText('Xalpha ｜beta《》');

    await page.keyboard.press('ControlOrMeta+Z');
    await expect(editor).toContainText('Xalpha beta');
    await page.keyboard.press('ControlOrMeta+Z');
    await expect(editor).toContainText('alpha beta');
  });

  test('shows human diagnostics and selects an astral-safe UTF-16 source range', async ({
    page,
  }) => {
    await replaceEditor(page, '😀》');
    await page.getByRole('radio', { name: 'Preview only' }).click();
    await expect(page.getByRole('region', { name: 'Editor' })).toBeHidden();
    const disclosure = page.getByRole('button', { name: 'Diagnostics (1)' });
    await expect(disclosure).toHaveAttribute('aria-expanded', 'true');
    const diagnostic = page.getByRole('button', {
      name: /unmatched.*aozora/i,
    });
    await expect(diagnostic).toBeVisible();
    await expectNoAxeViolations(page);
    await diagnostic.click();
    await expect(page.getByRole('region', { name: 'Editor' })).toBeVisible();
    await expect(page.locator('.cm-content')).toBeFocused();
    expect(await page.evaluate(() => getSelection()?.toString())).toBe('》');
  });

  test('reanalyzes localized outline fallbacks when language changes', async ({
    page,
  }) => {
    await replaceEditor(page, '#');
    await page.getByRole('button', { name: 'Outline' }).click();
    await expect(
      page.getByRole('button', {
        name: '(untitled), heading level 1',
      }),
    ).toBeVisible();

    await page.getByRole('button', { name: 'Settings' }).click();
    const settings = page.getByRole('dialog', { name: 'Settings' });
    await settings.getByRole('button', { name: /Language/ }).click();
    await page.getByRole('option', { name: 'Japanese' }).click();
    await page.keyboard.press('Escape');

    await expect(
      page.getByRole('button', {
        name: '（無題）、見出しレベル 1',
      }),
    ).toBeVisible();
  });

  test('changes writing direction without replacing the rendered document', async ({
    page,
  }) => {
    const heading = page.locator('.playground-preview-host h1');
    const before = await heading.textContent();
    const sameHeading = await heading.elementHandle();
    expect(sameHeading).not.toBeNull();
    await page.getByRole('radio', { name: 'Vertical' }).click();
    await expect(page.locator('.playground-preview-host')).toHaveAttribute(
      'data-writing-direction',
      'vertical',
    );
    await expect(page.locator('html')).toHaveAttribute(
      'data-aozora-md-theme',
      'vertical',
    );
    await expect(heading).toHaveText(before ?? '');
    const updatedHeading = await heading.elementHandle();
    expect(
      await sameHeading?.evaluate(
        (original, updated) => original.isSameNode(updated),
        updatedHeading,
      ),
    ).toBe(true);
  });

  test('does not mutate the URL while typing and creates a reloadable hash only on Share', async ({
    context,
    page,
  }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await replaceEditor(page, '# 共有する原稿');
    await expect(page).not.toHaveURL(/#src=/);
    await page.getByRole('button', { name: 'Share' }).click();
    await expect(page).toHaveURL(/#src=/);
    expect(await page.evaluate(() => navigator.clipboard.readText())).toContain(
      '#src=',
    );

    await page.reload();
    await expect(page.locator('.cm-content')).toContainText('共有する原稿');
  });

  test('persists a product draft and shared display preferences across reloads', async ({
    page,
  }) => {
    await replaceEditor(page, '# 保存された下書き');
    await page.getByRole('radio', { name: 'Preview only' }).click();
    await page.getByRole('button', { name: 'Outline' }).click();
    await page.getByRole('radio', { name: 'Vertical' }).click();
    await page.waitForTimeout(350);
    await page.reload();

    await expect(page.locator('.playground-preview-host h1')).toHaveText(
      '保存された下書き',
    );
    await expect(
      page.getByRole('complementary', { name: 'Outline' }),
    ).toBeVisible();
    await expect(page.getByRole('region', { name: 'Editor' })).toBeHidden();
    await expect(page.locator('.playground-preview-host')).toHaveAttribute(
      'data-writing-direction',
      'vertical',
    );
  });

  test('opens guide, settings, and About with focus restoration', async ({
    page,
  }) => {
    const guide = page.getByRole('button', { name: 'Guide' });
    await guide.click();
    await expect(
      page.getByRole('dialog', { name: 'aozora-md notation guide' }),
    ).toBeVisible();
    await expectNoAxeViolations(page);
    await page.keyboard.press('Escape');
    await expect(guide).toBeFocused();

    const settings = page.getByRole('button', { name: 'Settings' });
    await settings.click();
    await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
    await expect(
      page.getByRole('switch', { name: 'Structure highlighting' }),
    ).toBeChecked();
    await page.keyboard.press('Escape');
    await expect(settings).toBeFocused();

    const about = page.getByRole('button', {
      name: 'About this playground',
    });
    await about.click();
    const aboutDialog = page.getByRole('dialog', {
      name: 'About this playground',
    });
    await expect(aboutDialog).toContainText('Engine:');
    await expect(
      aboutDialog.getByRole('link', { name: 'Repository' }),
    ).toHaveAttribute(
      'href',
      'https://github.com/P4suta/aozora-flavored-markdown',
    );
    await expectNoAxeViolations(page);
  });

  test('loads a sample and persists language, theme, and editor assists', async ({
    page,
  }) => {
    const samplePicker = page.getByRole('button', { name: 'Sample' });
    const lightEditorColors = await page
      .locator('.cm-editor')
      .evaluate((node) => {
        const style = getComputedStyle(node);
        return {
          background: style.backgroundColor,
          color: style.color,
        };
      });
    await samplePicker.click();
    await page.getByRole('option', { name: 'Ruby and furigana' }).click();
    await expect(page.locator('.playground-preview-host h1')).toHaveText(
      'ルビ (振り仮名) いろいろ',
    );
    await expect(samplePicker).toContainText('Ruby and furigana');

    await page.getByRole('button', { name: 'Settings' }).click();
    const settings = page.getByRole('dialog', { name: 'Settings' });
    await settings.getByRole('button', { name: /Theme/ }).click();
    await page.getByRole('option', { name: 'Dark' }).click();
    await expect(page.locator('html')).toHaveAttribute(
      'data-color-scheme',
      'dark',
    );
    const darkEditorColors = await page
      .locator('.cm-editor')
      .evaluate((node) => {
        const style = getComputedStyle(node);
        return {
          background: style.backgroundColor,
          color: style.color,
          colorScheme: style.colorScheme,
        };
      });
    expect(darkEditorColors.colorScheme).toContain('dark');
    expect(darkEditorColors.background).not.toBe(lightEditorColors.background);
    expect(darkEditorColors.color).not.toBe(lightEditorColors.color);
    await settings.getByRole('button', { name: /Language/ }).click();
    const japaneseOption = page.getByRole('option', { name: 'Japanese' });
    await japaneseOption.click();
    await expect(japaneseOption).toBeHidden();

    const structureHighlight = page.getByRole('switch', {
      name: '構造ハイライト',
    });
    await structureHighlight.focus();
    await page.keyboard.press('Space');
    await expect(structureHighlight).not.toBeChecked();
    await page.keyboard.press('Escape');
    await page.reload();

    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
    await expect(page.locator('html')).toHaveAttribute(
      'data-color-scheme',
      'dark',
    );
    await page.getByRole('button', { name: '設定' }).click();
    await expect(
      page.getByRole('switch', { name: '構造ハイライト' }),
    ).not.toBeChecked();
  });

  test('keeps sample selection until the author edits the document', async ({
    page,
  }) => {
    const samplePicker = page.getByRole('button', { name: 'Sample' });
    await samplePicker.click();
    await page.getByRole('option', { name: 'Ruby and furigana' }).click();
    await expect(samplePicker).toContainText('Ruby and furigana');

    await replaceEditor(page, '# Author edit');
    await expect(samplePicker).not.toContainText('Ruby and furigana');
  });

  test('has no WCAG 2.2 AA violations in the primary and modal states', async ({
    page,
  }) => {
    await expectNoAxeViolations(page);
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
    await expectNoAxeViolations(page);
  });

  test('keeps the Japanese dark workspace and settings dialog accessible', async ({
    page,
  }) => {
    await page.evaluate(() => {
      localStorage.setItem(
        'aozora-playground:preferences:v2',
        JSON.stringify({
          colorScheme: 'dark',
          locale: 'ja',
          layout: 'split',
          writingDirection: 'horizontal',
          outlineOpen: false,
        }),
      );
    });
    await page.reload();

    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
    await expect(page.locator('html')).toHaveAttribute(
      'data-color-scheme',
      'dark',
    );
    await expectNoAxeViolations(page);
    await page.getByRole('button', { name: '設定' }).click();
    await expect(page.getByRole('dialog', { name: '設定' })).toBeVisible();
    await expectNoAxeViolations(page);
  });

  test('ships a stable desktop visual state', async ({ page }) => {
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot('desktop-default.png', {
      animations: 'disabled',
      maxDiffPixelRatio: 0.01,
    });
  });
});

test.describe('mobile authoring workspace', () => {
  test.use({ viewport: { width: 320, height: 720 } });

  test('switches editor and preview using tabs without page scrolling', async ({
    page,
  }) => {
    await openPlayground(page);
    await expect(page.getByRole('tab', { name: 'Editor' })).toBeVisible();
    await page.getByRole('tab', { name: 'Preview' }).click();
    await expect(page.locator('.playground-preview-host h1')).toBeVisible();
    await expect(
      page.getByRole('button', { name: /Diagnostics/ }),
    ).toBeVisible();

    const dimensions = await page.evaluate(() => ({
      body: document.body.scrollHeight,
      document: document.documentElement.scrollHeight,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
      viewport: window.innerHeight,
    }));
    expect(dimensions.body).toBeLessThanOrEqual(dimensions.viewport);
    expect(dimensions.document).toBeLessThanOrEqual(dimensions.viewport);
    expect(dimensions.overflow).toBeLessThanOrEqual(1);
  });

  test('retains the live editor and authoring state across mobile tabs and responsive layouts', async ({
    page,
  }) => {
    await openPlayground(page);
    await expect(
      page.getByRole('button', { name: 'Diagnostics' }),
    ).toBeEnabled();
    const editor = page.locator('.cm-content');
    const editorShell = page.locator('.cm-editor');
    const originalEditor = await editorShell.elementHandle();
    expect(originalEditor).not.toBeNull();

    await replaceEditor(
      page,
      [
        '# alpha',
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

    await page.getByRole('tab', { name: 'Preview' }).click();
    await expect(page.getByRole('tab', { name: 'Preview' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await expect(page.locator('.playground-preview-host h1')).toHaveText(
      'alpha',
    );
    await expect(page.locator('.playground-preview-host')).toContainText(
      'folded body',
    );
    await expectNoAxeViolations(page);

    await page.getByRole('tab', { name: 'Editor' }).click();
    await editor.focus();
    await expect(page.locator('.cm-foldPlaceholder')).toBeVisible();
    await expect(editor).not.toContainText('folded body');
    expect(await page.evaluate(() => getSelection()?.toString())).toBe(
      'omega!',
    );
    let currentEditor = await editorShell.elementHandle();
    expect(
      await originalEditor?.evaluate(
        (original, current) => original.isSameNode(current),
        currentEditor,
      ),
    ).toBe(true);

    await page.setViewportSize({ width: 1440, height: 900 });
    await expect(page.getByRole('radio', { name: 'Split' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Editor' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Preview' })).toBeVisible();
    await editor.focus();
    await expect(page.locator('.cm-foldPlaceholder')).toBeVisible();
    await expect(editor).not.toContainText('folded body');
    expect(await page.evaluate(() => getSelection()?.toString())).toBe(
      'omega!',
    );
    currentEditor = await editorShell.elementHandle();
    expect(
      await originalEditor?.evaluate(
        (original, current) => original.isSameNode(current),
        currentEditor,
      ),
    ).toBe(true);

    await page.setViewportSize({ width: 320, height: 720 });
    await expect(page.getByRole('tab', { name: 'Editor' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await editor.focus();
    await expect(page.locator('.cm-foldPlaceholder')).toBeVisible();
    await expect(editor).not.toContainText('folded body');
    expect(await page.evaluate(() => getSelection()?.toString())).toBe(
      'omega!',
    );
    currentEditor = await editorShell.elementHandle();
    expect(
      await originalEditor?.evaluate(
        (original, current) => original.isSameNode(current),
        currentEditor,
      ),
    ).toBe(true);

    await page.keyboard.press('ControlOrMeta+Z');
    await expect(editor).toContainText('omega');
    await expect(editor).not.toContainText('omega!');
    await expect(page.locator('.cm-foldPlaceholder')).toBeVisible();

    await editor.press('Control+Home');
    await editor.press('ArrowDown');
    await editor.press('Control+Shift+BracketRight');
    await expect(page.locator('.cm-foldPlaceholder')).toHaveCount(0);
    await expect(editor).toContainText('folded body');
  });

  test('uses mobile dialogs for outline and diagnostics and remains accessible', async ({
    page,
  }) => {
    await openPlayground(page);
    const outlineButton = page.getByRole('button', { name: 'Outline' });
    await outlineButton.click();
    const outlineDialog = page.getByRole('dialog', { name: 'Outline' });
    await expect(outlineDialog).toBeVisible();
    await expectNoAxeViolations(page);
    await page.keyboard.press('Escape');
    await expect(outlineDialog).toBeHidden();
    await expect(outlineButton).toBeFocused();

    await replaceEditor(page, '😀》');
    await page.getByRole('button', { name: 'Diagnostics (1)' }).click();
    const dialog = page.getByRole('dialog', { name: 'Diagnostics (1)' });
    await expect(dialog).toBeVisible();
    await expectNoAxeViolations(page);
    await dialog.getByRole('button', { name: /unmatched.*aozora/i }).click();
    await expect(page.getByRole('tab', { name: 'Editor' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await expect(page.locator('.cm-content')).toBeFocused();
  });

  test('ships a stable 320px visual state', async ({ page }) => {
    await openPlayground(page);
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot('mobile-320.png', {
      animations: 'disabled',
      maxDiffPixelRatio: 0.01,
    });
  });
});

test.describe('boot compatibility and production policy', () => {
  test('restores old aozora text and compressed query URLs', async ({
    page,
  }) => {
    const text = '# 旧 text URL';
    const encodedText = Buffer.from(text, 'utf8')
      .toString('base64')
      .replaceAll('+', '-')
      .replaceAll('/', '_')
      .replace(/=+$/, '');
    await page.goto(`./?text=${encodedText}`);
    await expect(page.locator('.cm-content')).toContainText('旧 text URL');

    const compressed = LZString.compressToBase64('# 旧 compressed URL')
      .replaceAll('+', '-')
      .replaceAll('/', '_')
      .replace(/=+$/, '');
    await page.goto(`./?c=${compressed}`);
    await expect(page.locator('.cm-content')).toContainText(
      '旧 compressed URL',
    );
  });

  test('gives an explicit shared hash priority over the saved product draft', async ({
    page,
  }) => {
    await page.addInitScript(() => {
      localStorage.setItem(
        'aozora-playground:draft:v1:aozora-flavored-markdown',
        '# local draft',
      );
    });
    const source = '# shared source';
    await page.goto(`./#src=${LZString.compressToEncodedURIComponent(source)}`);
    await expect(page.locator('.cm-content')).toContainText('shared source');
    await expect(page.locator('.cm-content')).not.toContainText('local draft');
  });

  test('migrates old shared display preferences', async ({ page }) => {
    await page.addInitScript(() => {
      localStorage.setItem('aozora-md-playground:color-scheme', 'dark');
      localStorage.setItem('aozora-md-playground:theme-mode', 'vertical');
      localStorage.setItem('aozora-playground:locale', 'ja');
    });
    await page.goto('./');
    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
    await expect(page.locator('html')).toHaveAttribute(
      'data-color-scheme',
      'dark',
    );
    await expect(page.getByRole('button', { name: '共有' })).toBeVisible();
    await expect(page.locator('.playground-preview-host')).toHaveAttribute(
      'data-writing-direction',
      'vertical',
    );
  });

  test('keeps the production CSP, base path, and every subresource self-hosted', async ({
    page,
  }) => {
    const origins = new Set<string>();
    page.on('request', (request) => origins.add(new URL(request.url()).origin));
    await openPlayground(page);

    const csp = await page
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute('content');
    expect(csp).toContain("default-src 'self'");
    expect(csp).toContain("script-src 'self' 'wasm-unsafe-eval'");
    expect(csp).toContain("object-src 'none'");
    expect(csp).not.toContain('http:');
    expect(csp).not.toContain('frame-ancestors');
    const scriptSource = await page
      .locator('script[type="module"]')
      .getAttribute('src');
    expect(scriptSource).toMatch(
      /^\/aozora-flavored-markdown\/playground\/assets\//,
    );
    await expect(page.locator('link[rel="canonical"]')).toHaveAttribute(
      'href',
      'https://p4suta.github.io/aozora-flavored-markdown/playground/',
    );
    await expect(page.locator('meta[property="og:url"]')).toHaveAttribute(
      'content',
      'https://p4suta.github.io/aozora-flavored-markdown/playground/',
    );
    await expect(page.locator('meta[name="twitter:card"]')).toHaveAttribute(
      'content',
      'summary',
    );
    expect([...origins]).toEqual([new URL(page.url()).origin]);

    const asset = await page.request.get(
      new URL(scriptSource ?? '', page.url()).href,
      { headers: { 'Accept-Encoding': 'gzip' } },
    );
    expect(asset.status()).toBe(200);
    expect(asset.headers()['cache-control']).toContain('immutable');
    expect(asset.headers()['content-encoding']).toBe('gzip');
  });
});
