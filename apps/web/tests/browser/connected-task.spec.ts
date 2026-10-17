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
  planning_capability?: string;
  planning_expect?: string[];
}

const path = process.env.MILKDRIFT_BROWSER_CONFIG;
if (!path) throw new Error('Set MILKDRIFT_BROWSER_CONFIG to the private browser.json produced by the Rust connected-task evidence harness.');
const config = JSON.parse(readFileSync(path, 'utf8')) as BrowserFixture;

async function openDaemonNavigation(page: Page) {
  const navigation = page.getByRole('button', { name: 'Daemon navigation', exact: true });
  if (await navigation.isVisible() && await navigation.getAttribute('aria-expanded') === 'false') await navigation.click();
}

async function connect(page: Page, daemon: DaemonFixture) {
  await openDaemonNavigation(page);
  await page.getByRole('button', { name: 'Add daemon connection' }).click();
  await page.getByLabel('Daemon URL').fill(daemon.url);
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
  await expect(page.getByText(/^Captured .+ · [\d,]+ bytes$/)).toBeVisible();
}

async function submit(page: Page, title: string, inputPath: string, deferBrief = false) {
  await fillTask(page, title, inputPath, deferBrief);
  await page.getByRole('button', { name: 'Submit task', exact: true }).click();
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
}

test('the branded workspace keeps empty discovery, connection and settings usable at narrow widths', async ({ page }, testInfo) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Connect to your work', exact: true })).toBeVisible();
  await page.keyboard.press('Tab');
  const skip = page.getByRole('link', { name: 'Skip to workspace', exact: true });
  await expect(skip).toBeFocused();
  await expect(skip).toBeVisible();
  const logo = page.locator('img.brand-logo');
  await expect(logo).toHaveAttribute('src', '/milkdrift-logo.svg');
  await expect.poll(() => logo.evaluate((element) => (element as HTMLImageElement).complete && (element as HTMLImageElement).naturalWidth > 0)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('empty-workspace-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole('button', { name: 'Connect a daemon', exact: true }).click();
  await expect(page.getByLabel('Daemon URL')).toBeVisible();
  await expect(page.getByLabel('Bearer credential')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('connection-phone.png'), fullPage: true });
  await page.getByLabel('Daemon URL').fill(config.b.url);
  await page.getByLabel('Bearer credential').fill(config.b.token);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'A clear space for your next task', exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('empty-discovery-phone.png'), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({ path: testInfo.outputPath('empty-discovery-desktop.png'), fullPage: true });
  await page.getByRole('button', { name: 'Daemon settings', exact: true }).click();
  await expect(page.getByLabel('Daemon name')).toHaveValue('Studio B');
  await page.getByText('Connection details', { exact: true }).click();
  await expect(page.getByText(config.b.identity, { exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('daemon-settings-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('daemon-settings-phone.png'), fullPage: true });
});

test('a real uploaded project produces discoverable input-dependent results after the browser closes', async ({ browser }, testInfo) => {
  const title = `Cargo analysis ${Date.now()}`;
  const first = await browser.newContext();
  const initial = await first.newPage();
  await initial.goto('/');
  await connect(initial, config.a);
  await submit(initial, title, config.input_a);
  await expect(initial.getByLabel('Task dependency timeline').getByText('Running', { exact: true })).toBeVisible();
  await first.close();

  const fresh = await browser.newContext();
  const page = await fresh.newPage();
  await page.goto('/');
  expect(await page.evaluate(() => window.localStorage.length)).toBe(0);
  await connect(page, config.a);
  await page.getByRole('button', { name: new RegExp(title) }).click();
  const output = page.getByTestId('result-output');
  await expect(output).toBeVisible({ timeout: 30000 });
  for (const expected of config.expect_a ?? ['dependencies']) await expect(output).toContainText(expected);
  const firstResult = await output.innerText();
  await page.getByText('Immutable result', { exact: true }).click();
  await expect(output).toBeHidden();
  await page.keyboard.press('Enter');
  await expect(output).toBeVisible();
  await expect(output).toHaveText(firstResult);
  console.log('CHECKPOINT: Production Svelte submitted the uploaded Cargo task, observed Running, disposed the client, then a fresh browser discovered its retained input-dependent result.');
  await page.getByRole('button', { name: 'List', exact: true }).click();
  await expect(page.getByRole('list').last()).toContainText('Completed');
  const completedBadge = page.locator('.task-list .badge').first();
  await expect(completedBadge).toHaveText('Completed');
  expect(await completedBadge.evaluate((element) => {
    const label = document.createRange();
    label.selectNodeContents(element);
    return label.getClientRects().length;
  })).toBe(1);
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
  expect(await page.locator('.task-list button').first().evaluate((element) => element.querySelector('.badge')!.getBoundingClientRect().right <= element.getBoundingClientRect().right)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('phone-result.png'), fullPage: true });

  await connect(page, config.b);
  await page.getByRole('searchbox').fill(title);
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'No matching workflows', exact: true })).toBeVisible();
  await openDaemonNavigation(page);
  await page.getByRole('button', { name: /^Studio A/ }).click();
  await expect(page.getByRole('heading', { name: variedTitle, level: 1, exact: true })).toBeVisible();
  const persisted = await page.evaluate(() => JSON.stringify({ local: { ...window.localStorage }, session: { ...window.sessionStorage } }));
  expect(persisted.includes(config.a.token) || persisted.includes(config.b.token)).toBe(false);
  await openDaemonNavigation(page);
  await page.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await expect(page.getByTestId('result-output')).toHaveCount(0);
  await fresh.close();
});

test('a queued missing-input task can be edited and repaired through the same inspector', async ({ page }) => {
  const title = `Queued Cargo analysis ${Date.now()}`;
  await page.goto('/');
  await connect(page, config.a);
  await submit(page, title, config.input_b, true);
  await expect(page.getByLabel('Task dependency timeline').getByText('Queued', { exact: true })).toBeVisible();
  await expect(page.getByText('Not started', { exact: true }).first()).toBeVisible();
  // The queued reason is a durable generation change. Edit the observed waiting
  // snapshot after it arrives, rather than racing that first scheduler decision.
  await expect(page.getByLabel('Task dependency timeline')).toContainText('missing supplied input brief');
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

test('the responsive inspector preserves a long-title task draft and keyboard focus while hidden', async ({ page }, testInfo) => {
  const title = `Animation research across rendering, learned motion, training data and validation boundaries ${Date.now()}`;
  const draft = 'Keep this unsaved instruction while inspecting the same queued work on a narrow screen.';
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto('/');
  await connect(page, config.a);
  await fillTask(page, title, config.input_a, true);
  await page.screenshot({ path: testInfo.outputPath('task-authoring-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('task-authoring-phone.png'), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole('button', { name: 'Submit task', exact: true }).click();
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
  const timeline = page.getByLabel('Task dependency timeline');
  await expect(timeline).toContainText('missing supplied input brief');
  const inspector = page.locator('.task-inspector');
  await expect(inspector.getByRole('heading', { name: title, exact: true })).toBeVisible();
  const desktop = await Promise.all([page.locator('.task-workspace').boundingBox(), inspector.boundingBox()]);
  expect(desktop[0]).not.toBeNull();
  expect(desktop[1]).not.toBeNull();
  expect(desktop[1]!.x).toBeGreaterThanOrEqual(desktop[0]!.x + desktop[0]!.width);
  await page.screenshot({ path: testInfo.outputPath('queued-long-title-desktop.png'), fullPage: true });

  await page.getByRole('button', { name: 'Edit unstarted task', exact: true }).click();
  const instructions = page.getByLabel('Instructions', { exact: true });
  await instructions.fill(draft);
  await page.setViewportSize({ width: 390, height: 844 });
  const stacked = await Promise.all([page.locator('.task-workspace').boundingBox(), inspector.boundingBox()]);
  expect(stacked[0]).not.toBeNull();
  expect(stacked[1]).not.toBeNull();
  expect(stacked[1]!.y).toBeGreaterThanOrEqual(stacked[0]!.y + stacked[0]!.height);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await instructions.focus();
  await page.keyboard.press('Escape');
  const reopen = page.getByRole('button', { name: 'Show inspector', exact: true });
  await expect(reopen).toBeFocused();
  await expect(reopen).toHaveAttribute('aria-expanded', 'false');
  await expect(instructions).toBeHidden();
  await page.keyboard.press('Enter');
  await expect(instructions).toBeFocused();
  await expect(instructions).toHaveValue(draft);
  const visibleFocus = await instructions.evaluate((element) => {
    const style = getComputedStyle(element);
    return style.outlineStyle !== 'none' && Number.parseFloat(style.outlineWidth) > 0;
  });
  expect(visibleFocus).toBe(true);
  await page.getByRole('button', { name: 'List', exact: true }).click();
  await page.locator('.task-list').getByRole('button').filter({ has: page.getByRole('heading', { name: title, exact: true }) }).click();
  await expect(instructions).toHaveValue(draft);
  await expect(inspector.getByText('Not started', { exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('queued-inspector-draft-phone.png'), fullPage: true });

  await page.evaluate(() => { document.documentElement.style.fontSize = '200%'; });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await expect(instructions).toHaveValue(draft);
  await expect(page.getByRole('button', { name: 'Save future task', exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('queued-inspector-enlarged-text.png'), fullPage: true });
});

test('queued cancellation permits linked replacement without inventing original execution times', async ({ page }) => {
  await page.goto('/');
  await connect(page, config.a);
  await submit(page, `Queued stop ${Date.now()}`, config.input_a, true);
  await expect(page.getByLabel('Task dependency timeline')).toContainText('missing supplied input brief');
  await page.getByRole('button', { name: 'Request stop', exact: true }).click();
  await expect(page.getByLabel('Task dependency timeline').getByText('Stopped', { exact: true })).toBeVisible();
  await expect(page.getByText('Not started', { exact: true }).first()).toBeVisible();
  await expect(page.getByTestId('result-output')).toHaveCount(0);
  await page.getByText('Task details and dependencies', { exact: true }).click();
  const originalId = await page.getByTestId('task-id').innerText();
  await page.getByRole('button', { name: 'Create linked corrective work', exact: true }).click();
  await page.getByRole('button', { name: 'Stage change', exact: true }).click();
  await page.getByLabel('Change to stage').selectOption('bind_input');
  await page.getByLabel('Supplied workflow input name').fill('brief');
  await page.getByLabel('Supplied input text').fill('Run the replacement against the original captured manifest after the stop settled.');
  await page.getByRole('button', { name: 'Stage change', exact: true }).click();
  await page.getByRole('button', { name: 'Preview complete batch', exact: true }).click();
  const batch = JSON.parse(await page.getByTestId('batch-preview').innerText());
  expect(batch.changes[0].definition).toMatchObject({ corrective_of: originalId, dependencies: [] });
  await page.getByRole('button', { name: 'Submit reviewed batch', exact: true }).click();
  const timeline = page.getByLabel('Task dependency timeline');
  const replacement = timeline.getByRole('button').filter({ has: page.getByRole('heading', { name: /^Correction: Queued stop / }) });
  await expect(replacement).toContainText('Completed', { timeout: 30000 });
  await replacement.click();
  await expect(page.getByTestId('result-output')).toBeVisible();
  await expect(timeline.getByText('Stopped', { exact: true })).toBeVisible();
  await expect(timeline.getByText('Not started', { exact: true })).toBeVisible();
});

test('an invalid supplied manifest leaves a readable failed task and blocked workflow', async ({ page }, testInfo) => {
  const title = `Invalid project manifest ${Date.now()}`;
  await page.goto('/');
  await connect(page, config.a);
  await fillTask(page, title, config.input_a);
  await page.getByLabel('Text file input').setInputFiles({
    name: 'Broken.toml', mimeType: 'text/plain', buffer: Buffer.from('[package\nname = "unfinished manifest"\n'),
  });
  await expect(page.getByText(/^Captured Broken.toml/)).toBeVisible();
  await page.getByRole('button', { name: 'Submit task', exact: true }).click();
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
  await expect(page.getByLabel('Task dependency timeline').getByText('Failed', { exact: true })).toBeVisible({ timeout: 30000 });
  await expect(page.locator('.workflow-header').getByText('blocked', { exact: true })).toBeVisible();
  const inspector = page.locator('.task-inspector');
  await expect(inspector.getByText('Failed', { exact: true })).toBeVisible();
  await expect(inspector.getByRole('button', { name: 'Create linked corrective work', exact: true })).toBeVisible();
  await expect(inspector.getByRole('button', { name: 'Edit unstarted task', exact: true })).toHaveCount(0);
  await expect(inspector.getByRole('button', { name: 'Request stop', exact: true })).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath('blocked-failure-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('blocked-failure-phone.png'), fullPage: true });
});

test('a reviewed future batch atomically revises a queued task and supplies its missing input', async ({ page }, testInfo) => {
  await page.goto('/');
  await connect(page, config.a);
  await submit(page, `Reviewed batch ${Date.now()}`, config.input_b, true);
  await expect(page.getByLabel('Task dependency timeline')).toContainText('missing supplied input brief');
  await page.getByText('Task details and dependencies', { exact: true }).click();
  const taskId = await page.getByTestId('task-id').innerText();
  await page.getByRole('button', { name: 'Edit future batch', exact: true }).click();
  await page.getByLabel('Change to stage').selectOption('revise');
  await page.getByLabel('Target task').selectOption(taskId);
  await page.getByRole('button', { name: 'Load observed task definition' }).click();
  await expect(page.getByText('Observed task version: 1', { exact: true })).toBeVisible();
  await page.getByLabel('Instructions', { exact: true }).fill('Read this captured manifest and report the actual runtime dependencies.');
  await page.getByRole('button', { name: 'Stage change', exact: true }).click();
  await page.getByLabel('Change to stage').selectOption('bind_input');
  await page.getByLabel('Supplied workflow input name').fill('brief');
  await page.getByLabel('Supplied input text').fill('Compare the runtime manifest after this reviewed batch is accepted.');
  await page.getByRole('button', { name: 'Stage change', exact: true }).click();
  let mutations = 0;
  page.on('request', (request) => { if (request.method() === 'POST' && request.url().endsWith('/changes')) mutations++; });
  await page.getByRole('button', { name: 'Preview complete batch', exact: true }).click();
  await expect(page.getByTestId('batch-preview')).toContainText('"kind": "revise"');
  await expect(page.getByTestId('batch-preview')).toContainText('"kind": "bind_input"');
  expect(mutations).toBe(0);
  await page.getByRole('region', { name: 'Exact batch request', exact: true }).focus();
  await page.keyboard.press('End');
  await expect.poll(() => page.getByTestId('batch-preview').evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await page.keyboard.press('Control+Home');
  await expect.poll(() => page.getByTestId('batch-preview').evaluate((element) => element.scrollTop)).toBe(0);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('reviewed-batch-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('reviewed-batch-phone.png'), fullPage: true });
  await page.getByRole('button', { name: 'Submit reviewed batch', exact: true }).click();
  await expect(page.getByTestId('result-output')).toBeVisible({ timeout: 30000 });
  expect(mutations).toBe(1);
  await page.getByText('Frozen start and selected inputs', { exact: true }).click();
  await expect(page.getByText('Compare the runtime manifest after this reviewed batch is accepted.', { exact: true })).toBeVisible();
  await expect(page.getByText('Read this captured manifest and report the actual runtime dependencies.', { exact: true })).toBeVisible();
});

test('a cyclic future batch is refused without changing accepted work or losing its draft', async ({ page }) => {
  await page.goto('/');
  await connect(page, config.a);
  await submit(page, `Refused batch ${Date.now()}`, config.input_b, true);
  await expect(page.getByLabel('Task dependency timeline')).toContainText('missing supplied input brief');
  await page.getByText('Task details and dependencies', { exact: true }).click();
  const taskId = await page.getByTestId('task-id').innerText();
  await page.getByRole('button', { name: 'Edit future batch', exact: true }).click();
  await page.getByLabel('Change to stage').selectOption('revise');
  await page.getByLabel('Target task').selectOption(taskId);
  await page.getByRole('button', { name: 'Load observed task definition' }).click();
  await expect(page.getByText('Observed task version: 1', { exact: true })).toBeVisible();
  await page.getByLabel('Instructions', { exact: true }).fill('This cyclic edit must never be accepted.');
  await page.getByLabel('Dependency task identity or batch alias').fill(taskId);
  await page.getByRole('button', { name: 'Add dependency', exact: true }).click();
  await page.getByRole('button', { name: 'Stage change', exact: true }).click();
  await page.getByRole('button', { name: 'Preview complete batch', exact: true }).click();
  await page.getByRole('button', { name: 'Submit reviewed batch', exact: true }).click();
  await expect(page.getByRole('alert').last()).toContainText(/cycle|cyclic|self/i);
  await expect(page.getByRole('heading', { name: 'Staged changes (1)' })).toBeVisible();
  await page.getByRole('button', { name: 'Close batch editor', exact: true }).click();
  await expect(page.getByText('This cyclic edit must never be accepted.', { exact: true })).toHaveCount(0);
  await expect(page.getByLabel('Task dependency timeline')).toContainText('Queued');
  await expect(page.getByTestId('result-output')).toHaveCount(0);
});

test('a lost accepted reply is recovered by the public receipt without resubmission', async ({ page }) => {
  await page.goto('/');
  await connect(page, config.a);
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
  await page.getByLabel('Daemon URL').fill('http://127.0.0.1:1');
  await page.getByLabel('Bearer credential').fill('non-secret-unreachable-test');
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('could not be reached');
  await expect(page.getByRole('button', { name: 'Connect', exact: true })).toBeEnabled();
  await expect(page.getByLabel('Bearer credential')).toHaveValue('');
});

test('the configured controlled model receives the same brief and captured file through the daemon', async ({ page }) => {
  test.skip(!config.model_capability, 'The Rust harness must provide the controlled model capability.');
  await page.goto('/');
  await connect(page, config.a);
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

test('the daemon admits model-authored parallel research and correction without browser continuation', async ({ page }, testInfo) => {
  if (!config.planning_capability) throw new Error('The Rust prompt-03 fixture must provide planning_capability for automatic-work evidence.');
  test.setTimeout(120000);
  await page.goto('/');
  await connect(page, config.a);
  let continuations = 0;
  page.on('request', (request) => { if (request.method() === 'POST' && request.url().endsWith('/changes')) continuations++; });
  const title = `Automatic animation research ${Date.now()}`;
  await page.getByRole('button', { name: '+ New task', exact: true }).click();
  await page.getByLabel('Task title', { exact: true }).fill(title);
  await page.getByLabel('Short description').fill('Parallel animation research with one explicit failed outcome and a successful adopted correction.');
  await page.getByLabel('Instructions', { exact: true }).fill('Plan parallel research for the supplied animation brief. Admit research and composition through structured actions, address the failed research prospectively, and explicitly finish after the final report.');
  await page.getByLabel('Brief input').fill('Develop a Rust approach to learned animation. Compare motion representation, training data and render validation in separate project folders, compose the evidence, and correct failed research without changing its original result.');
  await page.getByLabel('Capability', { exact: true }).selectOption({ label: `${config.planning_capability} · model.generate` });
  await page.getByLabel('Finish when required work settles').uncheck();
  await page.getByText('Model continuation and permitted tools', { exact: true }).click();
  await page.getByLabel('Allow model to propose future work').check();
  await page.getByLabel('Allow automatic feedback after refused proposals').check();
  await page.getByRole('button', { name: 'Submit task', exact: true }).click();
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
  const timeline = page.getByLabel('Task dependency timeline');
  await expect.poll(() => timeline.getByText('Running', { exact: true }).count(), { timeout: 30000 }).toBeGreaterThanOrEqual(2);
  await expect.poll(() => page.getByLabel('Actual task execution').getByRole('button', { name: /no end recorded$/ }).count()).toBeGreaterThanOrEqual(2);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('running-parallel-desktop.png'), fullPage: true });
  for (const child of config.planning_expect ?? ['Animation composition', 'Corrected render research', 'Adopt corrected research', 'Final animation report']) {
    await expect(timeline.getByRole('heading', { name: child, exact: true })).toBeVisible({ timeout: 90000 });
  }
  const finalTask = timeline.getByRole('button').filter({ has: page.getByRole('heading', { name: 'Final animation report', exact: true }) });
  await expect(finalTask).toContainText('Completed', { timeout: 90000 });
  await expect(page.locator('.workflow-header').getByText('completed', { exact: true })).toBeVisible();
  expect(continuations).toBe(0);
  const failedResearch = timeline.getByRole('button').filter({ has: page.getByRole('heading', { name: 'Research render validation', exact: true }) });
  await expect(failedResearch).toContainText('Failed');
  await failedResearch.click();
  await expect(page.locator('.task-inspector').getByText('Failed', { exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('retained-failure-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('retained-failure-phone.png'), fullPage: true });
  await page.setViewportSize({ width: 1280, height: 720 });
  const actual = page.getByLabel('Actual task execution');
  const motion = actual.getByRole('button', { name: /^Research motion representation: started/ });
  const training = actual.getByRole('button', { name: /^Research training data: started/ });
  const interval = async (bar: typeof motion) => bar.evaluate((element) => ({ left: parseFloat((element as HTMLElement).style.left), width: parseFloat((element as HTMLElement).style.width) }));
  const [motionInterval, trainingInterval] = await Promise.all([interval(motion), interval(training)]);
  expect(motionInterval.left).toBeLessThan(trainingInterval.left + trainingInterval.width);
  expect(trainingInterval.left).toBeLessThan(motionInterval.left + motionInterval.width);
  await timeline.getByRole('button').filter({ has: page.getByRole('heading', { name: 'Animation composition', exact: true }) }).click();
  await page.getByText('Accepted inputs, outputs and project paths', { exact: true }).click();
  await expect(page.getByText(/Retained outcome of .*including failure/)).toBeVisible();
  await page.getByText('Frozen start and selected inputs', { exact: true }).click();
  await expect(page.getByText(/Follow-up work admitted from task/)).toBeVisible();
  const responseText = page.getByRole('region', { name: 'Retained result text', exact: true });
  await responseText.focus();
  await page.keyboard.press('End');
  await expect.poll(() => responseText.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await page.keyboard.press('Control+Home');
  await expect.poll(() => responseText.evaluate((element) => element.scrollTop)).toBe(0);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('automatic-animation-desktop.png'), fullPage: true });
  await finalTask.click();
  await expect(page.getByTestId('result-output')).toBeVisible();
  await expect(page.getByTestId('result-output')).toHaveText(/^Final animation report/);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole('button', { name: 'List', exact: true }).click();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('automatic-animation-phone.png'), fullPage: true });
});

test('keyboard access and identity refusal are visible on a phone layout', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(({ endpoint }) => {
    window.localStorage.setItem('milkdrift.connections.v1', JSON.stringify([{ id: 'old-profile', label: 'Old studio', endpoint, daemonId: 'not-the-configured-installation' }]));
  }, { endpoint: config.a.url });
  await page.goto('/');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Skip to workspace' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Daemon navigation', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Daemon navigation', exact: true })).toHaveAttribute('aria-expanded', 'true');
  await page.getByRole('button', { name: /^Old studio/ }).click();
  await page.getByLabel('Bearer credential').fill(config.a.token);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('different daemon');
  await expect(page.getByRole('heading', { name: 'Your workflows' })).toHaveCount(0);
  expect(await page.evaluate(() => JSON.parse(window.localStorage.getItem('milkdrift.connections.v1')!)[0].daemonId)).toBe('not-the-configured-installation');
  await expect(page.getByLabel('Bearer credential')).toHaveValue('');
  await openDaemonNavigation(page);
  await page.getByRole('button', { name: 'Forget Old studio connection' }).click();
  await connect(page, config.a);
  await openDaemonNavigation(page);
  await expect(page.getByRole('button', { name: /^Studio A/ })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});

test('a browser at an unconfigured origin receives a useful connection refusal', async ({ page }) => {
  await page.goto('http://127.0.0.1:4174');
  await page.getByRole('button', { name: 'Connect a daemon', exact: true }).click();
  await page.getByLabel('Daemon URL').fill(config.a.url);
  await page.getByLabel('Bearer credential').fill(config.a.token);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('allowed browser origin');
  await expect(page.getByLabel('Bearer credential')).toHaveValue('');
  await expect(page.getByRole('heading', { name: 'Your workflows', exact: true })).toHaveCount(0);
});

test('independent clients share a recovered rename and distinguish duplicate names by daemon address', async ({ browser }, testInfo) => {
  const controller = await browser.newContext();
  const observer = await browser.newContext();
  const first = await controller.newPage();
  const second = await observer.newPage();
  await first.goto('/');
  await second.goto('/');
  await connect(first, config.a);
  await connect(second, config.a);
  await second.getByRole('searchbox').fill(`No workflow matches this ${Date.now()}`);
  await second.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(second.getByRole('heading', { name: 'No matching workflows' })).toBeVisible();
  await connect(first, config.b);
  await first.getByRole('button', { name: /^Studio A/ }).click();
  await first.getByRole('button', { name: 'Daemon settings', exact: true }).click();
  await expect(first.getByLabel('Daemon name')).toHaveValue('Studio A');
  let submitted = 0;
  await first.route('**/v1/daemon/rename', async (route) => {
    submitted++;
    const response = await route.fetch();
    expect(response.ok()).toBe(true);
    await route.abort('failed');
  });
  try {
    await first.getByLabel('Daemon name').fill('Studio B');
    await first.getByRole('button', { name: 'Save daemon name', exact: true }).click();
    await expect(first.getByRole('button', { name: 'Find rename receipt' })).toBeVisible();
    await first.getByRole('button', { name: 'Back to work' }).click();
    await first.getByRole('button', { name: 'Daemon settings', exact: true }).click();
    await expect(first.getByRole('button', { name: 'Find rename receipt' })).toBeVisible();
    await expect(second.getByRole('button', { name: /^Studio B/ })).toBeVisible();
    await first.getByRole('button', { name: 'Find rename receipt' }).click();
    await expect(first.getByRole('status')).toContainText('Name saved on this daemon');
    await expect(first.getByRole('button', { name: 'Save daemon name', exact: true })).toBeEnabled();
    expect(submitted).toBe(1);
    const duplicates = first.getByRole('button', { name: /^Studio B/ });
    await expect(duplicates).toHaveCount(2);
    await expect(duplicates.filter({ hasText: config.a.url })).toBeVisible();
    await expect(duplicates.filter({ hasText: config.b.url })).toBeVisible();
    await first.screenshot({ path: testInfo.outputPath('shared-name-desktop.png'), fullPage: true });
    await first.setViewportSize({ width: 390, height: 844 });
    await openDaemonNavigation(first);
    await expect(duplicates.filter({ hasText: config.a.url })).toContainText(config.a.url);
    await expect(duplicates.filter({ hasText: config.b.url })).toContainText(config.b.url);
    expect(await first.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await first.screenshot({ path: testInfo.outputPath('shared-name-phone.png'), fullPage: true });
    await second.reload();
    await expect(second.getByRole('button', { name: /^Studio B/ })).toContainText('Cached name');
    await second.getByRole('button', { name: /^Studio B/ }).click();
    await second.getByLabel('Bearer credential').fill(config.a.token);
    await second.getByRole('button', { name: 'Connect', exact: true }).click();
    await expect(second.getByRole('heading', { name: 'Your workflows' })).toBeVisible();
    await expect(second.getByRole('button', { name: /^Studio B/ })).toContainText('Connected');
    const fresh = await browser.newContext();
    try {
      const freshPage = await fresh.newPage();
      await freshPage.goto('/');
      await connect(freshPage, config.a);
      await expect(freshPage.getByRole('button', { name: /^Studio B/ })).toBeVisible();
    } finally { await fresh.close(); }
    const profiles = await first.evaluate(() => JSON.parse(window.localStorage.getItem('milkdrift.connections.v1')!));
    expect(profiles.map((profile: { daemonId: string }) => profile.daemonId)).toEqual([config.a.identity, config.b.identity]);
    expect(JSON.stringify(profiles).includes(config.a.token)).toBe(false);
    expect(JSON.stringify(profiles).includes(config.b.token)).toBe(false);
    await first.unroute('**/v1/daemon/rename');
    const longName = 'A'.repeat(80);
    await first.getByLabel('Daemon name').fill(longName);
    await first.getByRole('button', { name: 'Save daemon name', exact: true }).click();
    await expect(first.getByRole('button', { name: new RegExp(`^${longName}`) })).toBeVisible();
    expect(await first.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await first.getByRole('button', { name: 'Back to work', exact: true }).click();
    await first.getByRole('button', { name: '+ New task', exact: true }).click();
    await expect(first.getByRole('heading', { name: 'Start a task', exact: true })).toBeVisible();
    expect(await first.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await first.screenshot({ path: testInfo.outputPath('long-daemon-name-composer-phone.png'), fullPage: true });
    await first.getByRole('button', { name: 'Close task form', exact: true }).click();
    await openDaemonNavigation(first);
    await first.getByRole('button', { name: 'Daemon settings', exact: true }).click();
    await expect(first.getByLabel('Daemon name')).toHaveValue(longName);
  } finally {
    await first.unroute('**/v1/daemon/rename');
    // Leave the shared review fixture with its seeded human-readable name.
    if (!await first.getByLabel('Daemon name').count()) {
      await openDaemonNavigation(first);
      await first.getByRole('button', { name: 'Daemon settings', exact: true }).click();
    }
    if (await first.getByRole('button', { name: 'Find rename receipt' }).count()) {
      await first.getByRole('button', { name: 'Find rename receipt' }).click();
    }
    await first.getByLabel('Daemon name').fill('Studio A');
    await first.getByRole('button', { name: 'Save daemon name', exact: true }).click();
    await openDaemonNavigation(first);
    await expect(first.getByRole('button', { name: /^Studio A/ })).toBeVisible();
    await controller.close();
    await observer.close();
  }
});

test('a bad credential cannot install a saved profile or expose daemon work', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Connect a daemon', exact: true }).click();
  await page.getByLabel('Daemon URL').fill(config.a.url);
  await page.getByLabel('Bearer credential').fill('deliberately-invalid-browser-credential');
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  await expect(page.getByLabel('Bearer credential')).toHaveValue('');
  await expect(page.getByRole('heading', { name: 'Your workflows' })).toHaveCount(0);
  expect(await page.evaluate(() => window.localStorage.getItem('milkdrift.connections.v1'))).toBeNull();
  await page.getByLabel('Bearer credential').fill(config.a.token);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Your workflows' })).toBeVisible();
});
