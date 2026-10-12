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
  model_capability?: string;
  model_expect?: string[];
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

async function fillTask(page: Page, title: string, inputPath: string, deferBrief = false) {
  await page.getByRole('button', { name: '+ New task', exact: true }).click();
  await page.getByLabel('Task title', { exact: true }).fill(title);
  await page.getByLabel('Short description').fill('Inspect the supplied Cargo project and identify its targets and dependency choices.');
  await page.getByLabel('Instructions', { exact: true }).fill('Read the supplied Cargo.toml. Produce a substantive report of its package, targets, dependencies and workspace structure. State the limits of what the manifest establishes.');
  await page.getByLabel('Brief input').fill('Develop a Rust-based approach to learned animation. Use this real project manifest to identify the available runtime foundation and dependencies.');
  if (deferBrief) await page.getByLabel('Supply brief now').uncheck();
  await page.getByLabel('Capability', { exact: true }).selectOption({ label: `${config.capability} · process.execute` });
  await page.getByLabel('Text file input').setInputFiles(inputPath);
  await expect(page.getByText(/^Captured /)).toBeVisible();
}

async function submit(page: Page, title: string, inputPath: string, deferBrief = false) {
  await fillTask(page, title, inputPath, deferBrief);
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

test('a queued missing-input task can be edited and repaired through the same inspector', async ({ page }) => {
  const title = `Queued Cargo analysis ${Date.now()}`;
  await page.goto('/');
  await connect(page, config.a, 'Repair studio');
  await submit(page, title, config.input_b, true);
  await expect(page.getByLabel('Task dependency timeline').getByText('Queued', { exact: true })).toBeVisible();
  await expect(page.getByText('Not started', { exact: true }).first()).toBeVisible();
  await page.getByRole('button', { name: 'List', exact: true }).click();
  await page.getByRole('button', { name: 'Edit unstarted task', exact: true }).click();
  await page.getByLabel('Instructions', { exact: true }).fill('Inspect only the captured runtime Cargo manifest and report its actual dependency names.');
  await page.getByRole('button', { name: 'Save future task', exact: true }).click();
  await expect(page.getByText('Inspect only the captured runtime Cargo manifest and report its actual dependency names.', { exact: true })).toBeVisible();
  await page.getByText('Supply a required or replacement input', { exact: true }).click();
  await page.getByLabel('Workflow input', { exact: true }).selectOption('brief');
  await page.getByLabel('Input text', { exact: true }).fill('Supply the missing brief and start this queued analysis using its already captured file.');
  await page.getByRole('button', { name: 'Supply input', exact: true }).click();
  // The runtime may durably record a queued reason after the edit. A generation conflict
  // requires an explicit new decision against the refreshed snapshot, with the draft intact.
  await expect.poll(async () => (await page.getByTestId('result-output').count()) > 0 || (await page.getByRole('alert').allTextContents()).some((text) => text.includes('stale workflow generation')), { timeout: 30000 }).toBe(true);
  if ((await page.getByRole('alert').allTextContents()).some((text) => text.includes('stale workflow generation'))) {
    await expect(page.getByLabel('Input text', { exact: true })).toHaveValue('Supply the missing brief and start this queued analysis using its already captured file.');
    await page.getByRole('button', { name: 'Supply input', exact: true }).click();
  }
  await expect(page.getByTestId('result-output')).toBeVisible({ timeout: 30000 });
  for (const expected of config.expect_b ?? ['dependencies']) await expect(page.getByTestId('result-output')).toContainText(expected);
  await expect(page.getByRole('button', { name: 'Edit unstarted task', exact: true })).toHaveCount(0);
});

test('queued cancellation is a daemon outcome and does not invent execution times', async ({ page }) => {
  await page.goto('/');
  await connect(page, config.a, 'Stop studio');
  await submit(page, `Queued stop ${Date.now()}`, config.input_a, true);
  await page.getByRole('button', { name: 'Request stop', exact: true }).click();
  await expect(page.getByLabel('Task dependency timeline').getByText('Stopped', { exact: true })).toBeVisible();
  await expect(page.getByText('Not started', { exact: true }).first()).toBeVisible();
  await expect(page.getByTestId('result-output')).toHaveCount(0);
});

test('a lost accepted reply is recovered by the public receipt without resubmission', async ({ page }) => {
  await page.goto('/');
  await connect(page, config.a, 'Recovery studio');
  const title = `Lost reply ${Date.now()}`;
  await fillTask(page, title, config.input_a);
  let submitted = 0;
  await page.route('**/v1/workflows', async (route) => {
    if (route.request().method() !== 'POST') { await route.continue(); return; }
    submitted++;
    const actual = await route.fetch();
    expect(actual.ok()).toBe(true);
    await route.abort('failed');
  });
  await page.getByRole('button', { name: 'Submit task', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Find submission receipt', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Find submission receipt', exact: true }).click();
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
  await expect(page.getByTestId('result-output')).toBeVisible({ timeout: 30000 });
  expect(submitted).toBe(1);
});

test('an unreachable daemon leaves a retryable connection form with no retained credential', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Connect a daemon', exact: true }).click();
  await page.getByLabel('Connection name').fill('Unavailable');
  await page.getByLabel('Daemon URL').fill('http://127.0.0.1:1');
  await page.getByLabel('Installation identity').fill('host:unavailable');
  await page.getByLabel('Bearer credential').fill('non-secret-unreachable-test');
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('could not be reached');
  await expect(page.getByRole('button', { name: 'Connect', exact: true })).toBeEnabled();
  await expect(page.getByLabel('Bearer credential')).toHaveValue('');
});

test('the configured controlled model receives the same brief and captured file through the daemon', async ({ page }) => {
  test.skip(!config.model_capability, 'The Rust harness must provide the controlled model capability.');
  await page.goto('/');
  await connect(page, config.a, 'Model studio');
  const title = `Controlled model analysis ${Date.now()}`;
  await fillTask(page, title, config.input_b);
  await page.getByLabel('Capability', { exact: true }).selectOption({ label: `${config.model_capability} · model.generate` });
  await page.getByRole('button', { name: 'Submit task', exact: true }).click();
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
  await expect(page.getByTestId('result-output')).toBeVisible({ timeout: 30000 });
  for (const expected of config.model_expect ?? ['Controlled analysis']) await expect(page.getByTestId('result-output')).toContainText(expected);
  await page.getByText('Frozen start and selected inputs', { exact: true }).click();
  await expect(page.getByText('file · Cargo.toml', { exact: true })).toBeVisible();
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
