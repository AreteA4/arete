import { expect, test } from '@playwright/test';

test('desktop keeps the board, compact stats, and deployment form visible', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Ore Mining' })).toBeVisible();
  await expect(page.getByLabel('ORE squares 1 through 25')).toBeVisible();
  await expect(page.getByText('Motherlode', { exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Deployment' })).toBeVisible();
  await expect(page.getByTestId('tile-25')).toBeVisible();
  const dimensions = await page.evaluate(() => ({
    scrollHeight: document.documentElement.scrollHeight,
    clientHeight: document.documentElement.clientHeight,
  }));
  expect(dimensions.scrollHeight).toBeLessThanOrEqual(dimensions.clientHeight);
});

test('mobile stacks without overflow and preserves touch targets', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  const dimensions = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
    scrollHeight: document.documentElement.scrollHeight,
    clientHeight: document.documentElement.clientHeight,
  }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
  expect(dimensions.scrollHeight).toBeLessThanOrEqual(dimensions.clientHeight);
  const tile = page.getByTestId('tile-1');
  await expect(tile).toBeVisible();
  const box = await tile.boundingBox();
  expect(box?.height).toBeGreaterThanOrEqual(44);
  await tile.click();
  await expect(tile).toHaveAttribute('aria-pressed', 'true');
  const theme = page.getByRole('button', { name: /Switch to .* mode/ });
  const connect = page.getByRole('button', { name: 'Connect wallet to continue' });
  expect((await theme.boundingBox())?.height).toBeGreaterThanOrEqual(44);
  expect((await connect.boundingBox())?.height).toBeGreaterThanOrEqual(44);
  await expect(page.getByRole('heading', { name: 'Deployment' })).toBeVisible();
});

test('direct square selection and keyboard toggling work while disconnected', async ({ page }) => {
  await page.goto('/');
  const first = page.getByTestId('tile-1');
  await first.focus();
  await page.keyboard.press('Space');
  await expect(first).toHaveAttribute('aria-pressed', 'true');
  const second = page.getByTestId('tile-2');
  await second.click();
  await expect(page.locator('[data-testid^="tile-"][aria-pressed="true"]')).toHaveCount(2);
  await first.focus();
  await page.keyboard.press('Enter');
  await expect(first).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('[data-testid^="tile-"][aria-pressed="true"]')).toHaveCount(1);
});

test('transaction control prompts wallet connection and reduced motion remains usable', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/');
  await page.getByTestId('tile-1').click();
  await page.getByRole('button', { name: 'Connect wallet to continue' }).click();
  await expect(page.locator('.wallet-adapter-modal')).toBeVisible();
  await expect(page.getByTestId('tile-1')).toHaveAttribute('aria-pressed', 'true');
});
