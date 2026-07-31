import { expect, test } from '@playwright/test';
import LZString from 'lz-string';

test.describe('shared source coordinates', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('selects second-line ranges from a CRLF shared URL', async ({
    page,
  }) => {
    const source = '# First\r\n# Second\r\n》';
    const encoded = LZString.compressToEncodedURIComponent(source);
    await page.goto(`./#src=${encoded}`);

    await expect(page.locator('.playground-preview-host h1')).toHaveText([
      'First',
      'Second',
    ]);
    expect(new URL(page.url()).hash).toBe(`#src=${encoded}`);

    await page.getByRole('button', { name: 'Outline' }).click();
    const outline = page.getByRole('complementary', { name: 'Outline' });
    await outline
      .getByRole('button', {
        name: 'Second, heading level 1',
      })
      .click();
    expect(await page.evaluate(() => getSelection()?.toString())).toBe(
      '# Second',
    );

    const diagnostics = page.getByRole('button', {
      name: 'Diagnostics (1)',
    });
    await diagnostics.click();
    await page.getByRole('button', { name: /unmatched.*aozora/i }).click();
    expect(await page.evaluate(() => getSelection()?.toString())).toBe('》');

    await page.waitForTimeout(350);
    expect(
      await page.evaluate(() =>
        localStorage.getItem(
          'aozora-playground:draft:v1:aozora-flavored-markdown',
        ),
      ),
    ).toBe(source);
  });
});
