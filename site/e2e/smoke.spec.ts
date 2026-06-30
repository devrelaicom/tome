import {test, expect} from '@playwright/test';

test('landing renders the grimoire hero and all six sections', async ({page}) => {
  await page.goto('/');
  await expect(page.getByRole('heading', {level: 1, name: /Tome/})).toBeVisible();
  await expect(page.getByText('One bookshelf for everything your coding agents know')).toBeVisible();
  for (const t of [
    'Five agents. Five dialects. One you.',
    'Shelve it once. Read it everywhere.',
    'COLOPHON',
    'Already have plugins? Convert them.',
    'Add it to your bookshelf.',
  ]) {
    await expect(page.getByText(t, {exact: false}).first()).toBeVisible();
  }
});

test('terminal demo renders statically under reduced motion', async ({page}) => {
  await page.emulateMedia({reducedMotion: 'reduce'});
  await page.goto('/');
  await expect(page.getByText('tome catalog add devrelaicom/midnight-expert-tome').first()).toBeVisible();
  await expect(page.getByText('stayed on the shelf')).toBeVisible();
});

test('install command shown and nav reaches docs', async ({page}) => {
  await page.goto('/');
  await expect(page.getByText('cargo install tome-mcp')).toBeVisible();
  await page.getByRole('link', {name: 'Docs'}).first().click();
  await expect(page).toHaveURL(/\/docs\//);
});

test('docs sidebar shows the four chapters and navigates', async ({page}) => {
  await page.goto('/docs/getting-started/install');
  for (const ch of ['Ch. I — Getting started', 'Ch. II — Using Tome', 'Ch. III — Authoring', 'Ch. IV — Reference']) {
    await expect(page.getByText(ch).first()).toBeVisible();
  }
  await page.getByRole('link', {name: 'Converting'}).click();
  await expect(page).toHaveURL(/\/docs\/authoring\/convert/);
});

test('dark mode toggle switches to the midnight library', async ({page}) => {
  await page.emulateMedia({colorScheme: 'light'});
  await page.goto('/');
  // Docusaurus 3.10's toggle cycles system → light → dark; click until dark sticks.
  const toggle = page.getByRole('button', {name: /dark mode|color mode|switch between dark and light/i});
  for (let i = 0; i < 3; i += 1) {
    if ((await page.locator('html').getAttribute('data-theme')) === 'dark') break;
    await toggle.click();
  }
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
});
