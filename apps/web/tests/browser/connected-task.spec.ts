import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

interface DaemonFixture { url: string; identity: string; token: string }
interface BrowserFixture {
  a: DaemonFixture;
  b: DaemonFixture;
  capability: string;
  input_a: string;
  input_b: string;
  expect_a?: string[];
  expect_b?: string[];
}

const path = process.env.MILKDRIFT_BROWSER_CONFIG;
if (!path) throw new Error('Set MILKDRIFT_BROWSER_CONFIG to the private browser.json produced by the Rust connected-task evidence harness.');
const config = JSON.parse(readFileSync(path, 'utf8')) as BrowserFixture;

async function connect(page: Page, daemon: DaemonFixture, name: string) {
  await page.getByRole('button', { name: 'Add daemon connection' }).click();
  await page.getByLabel('Connection name').fill(name);
  await page.getByLabel('Daemon URL').fill(daemon.url);
  await page.getByLabel('Installation identity').fill(daemon.identity);
  await page.getByLabel('Bearer credential').fill(daemon.token);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Your workflows', exact: true })).toBeVisible();
}

async function submit(page: Page, title: string, inputPath: string) {
  await page.getByRole('button', { name: '+ New task', exact: true }).click();
  await page.getByLabel('Task title', { exact: true }).fill(title);
  await page.getByLabel('Short description').fill('Inspect the supplied Cargo project and identify its targets and dependency choices.');
  await page.getByLabel('Instructions', { exact: true }).fill('Read the supplied Cargo.toml. Produce a substantive report of its package, targets, dependencies and workspace structure. State the limits of what the manifest establishes.');
  await page.getByLabel('Brief input').fill('Develop a Rust-based approach to learned animation. Use this real project manifest to identify the available runtime foundation and dependencies.');
  await page.getByLabel('Capability', { exact: true }).selectOption({ label: `${config.capability} · process.execute` });
  await page.getByLabel('Text file input').setInputFiles(inputPath);
  await expect(page.getByText(/^Captured /)).toBeVisible();
  await page.getByRole('button', { name: 'Submit task', exact: true }).click();
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
}

test('a real uploaded project produces discoverable input-dependent results after the browser closes', async ({ browser }, testInfo) => {
  const title = `Cargo analysis ${Date.now()}`;
  const first = await browser.newContext();
  const initial = await first.newPage();
  await initial.goto('/');
  await connect(initial, config.a, 'Studio A');
  await submit(initial, title, config.input_a);
  await expect(initial.getByLabel('Task dependency timeline').getByText('Running', { exact: true })).toBeVisible();
  await first.close();

  const fresh = await browser.newContext();
  const page = await fresh.newPage();
  await page.goto('/');
  expect(await page.evaluate(() => window.localStorage.length)).toBe(0);
  await connect(page, config.a, 'Studio A');
  await page.getByRole('button', { name: new RegExp(title) }).click();
  const output = page.getByTestId('result-output');
  await expect(output).toBeVisible({ timeout: 30000 });
  for (const expected of config.expect_a ?? ['dependencies']) await expect(output).toContainText(expected);
  const firstResult = await output.innerText();
  console.log('CHECKPOINT: Production Svelte submitted the uploaded Cargo task, observed Running, disposed the client, then a fresh browser discovered its retained input-dependent result.');
  await page.getByRole('button', { name: 'List', exact: true }).click();
  await expect(page.getByRole('list').last()).toContainText('Completed');
  await page.screenshot({ path: testInfo.outputPath('desktop-result.png'), fullPage: true });

  await page.getByRole('button', { name: '← All workflows', exact: true }).click();
  const variedTitle = `${title} varied input`;
  await submit(page, variedTitle, config.input_b);
  await expect(output).toBeVisible({ timeout: 30000 });
  for (const expected of config.expect_b ?? ['dependencies']) await expect(output).toContainText(expected);
  expect(await output.innerText()).not.toBe(firstResult);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole('button', { name: 'List', exact: true }).click();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('phone-result.png'), fullPage: true });

  await connect(page, config.b, 'Studio B');
  await page.getByRole('searchbox').fill(title);
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'No matching workflows', exact: true })).toBeVisible();
  await page.getByRole('button', { name: /^Studio A/ }).click();
  await expect(page.getByRole('heading', { name: variedTitle, level: 1, exact: true })).toBeVisible();
  const persisted = await page.evaluate(() => JSON.stringify({ local: { ...window.localStorage }, session: { ...window.sessionStorage } }));
  expect(persisted.includes(config.a.token) || persisted.includes(config.b.token)).toBe(false);
  await page.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await expect(page.getByTestId('result-output')).toHaveCount(0);
  await fresh.close();
});

test('keyboard access and identity refusal are visible on a phone layout', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Skip to workspace' })).toBeFocused();
  await page.getByRole('button', { name: 'Connect a daemon', exact: true }).click();
  await page.getByLabel('Connection name').fill('Wrong identity');
  await page.getByLabel('Daemon URL').fill(config.a.url);
  await page.getByLabel('Installation identity').fill('not-the-configured-installation');
  await page.getByLabel('Bearer credential').fill(config.a.token);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Installation identity does not match');
  await expect(page.getByLabel('Bearer credential')).toHaveValue('');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});

test('a browser at an unconfigured origin receives a useful connection refusal', async ({ page }) => {
  await page.goto('http://127.0.0.1:4174');
  await page.getByRole('button', { name: 'Connect a daemon', exact: true }).click();
  await page.getByLabel('Connection name').fill('Denied origin');
  await page.getByLabel('Daemon URL').fill(config.a.url);
  await page.getByLabel('Installation identity').fill(config.a.identity);
  await page.getByLabel('Bearer credential').fill(config.a.token);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('allowed browser origin');
  await expect(page.getByLabel('Bearer credential')).toHaveValue('');
  await expect(page.getByRole('heading', { name: 'Your workflows', exact: true })).toHaveCount(0);
});
