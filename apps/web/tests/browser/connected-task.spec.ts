import { readFileSync, statSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
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
  planning_project_root: string;
}

const path = process.env.MILKDRIFT_BROWSER_CONFIG;
if (!path) throw new Error('Set MILKDRIFT_BROWSER_CONFIG to the private browser.json produced by the Rust connected-task evidence harness.');
const config = JSON.parse(readFileSync(path, 'utf8')) as BrowserFixture;

async function openDaemonNavigation(page: Page) {
  const navigation = page.getByRole('button', { name: 'Workspace navigation', exact: true });
  if (await navigation.isVisible() && await navigation.getAttribute('aria-expanded') === 'false') await navigation.click();
}

async function openSavedConnections(page: Page) {
  await openDaemonNavigation(page);
  const saved = page.locator('.saved-connections');
  if (await saved.getAttribute('open') === null) await saved.getByText('Manage connections', { exact: true }).click();
}

async function connect(page: Page, daemon: DaemonFixture) {
  await openDaemonNavigation(page);
  await page.getByRole('button', { name: 'Add daemon connection' }).click();
  await page.getByLabel('Daemon URL').fill(daemon.url);
  await page.getByLabel('Bearer credential').fill(daemon.token);
  const projectsUrl = new URL('/v1/projects', daemon.url).href;
  const projectsLoaded = page.waitForResponse((response) => response.url().startsWith(`${projectsUrl}?`) && response.request().method() === 'GET');
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Your projects', exact: true })).toBeVisible();
  const initialProjects = (await (await projectsLoaded).json()) as { value: { items: Array<{ name: string }> } };
  const project = page.getByRole('button', { name: 'Open project Animation project', exact: true });
  if (!initialProjects.value.items.some((item) => item.name === 'Animation project')) {
    await page.locator('main').getByRole('button', { name: 'Add project', exact: true }).click();
    await page.getByRole('button', { name: 'Browse permitted folders', exact: true }).click();
    await page.locator('.folder-chooser').getByRole('button', { name: /^daemon-[ab]$/ }).click();
    await page.getByRole('button', { name: 'Use animation-project', exact: true }).click();
    await page.getByLabel('Project name', { exact: true }).fill('Animation project');
    await page.getByRole('button', { name: 'Attach project', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Project settings', exact: true })).toBeVisible();
    await page.getByRole('checkbox', { name: 'Allow project changes', exact: true }).check();
    await page.getByRole('button', { name: 'Save project settings', exact: true }).click();
    await expect(page.getByRole('status')).toContainText('Saved on this daemon.');
    await page.getByRole('button', { name: 'Back to work', exact: true }).click();
  } else {
    await openDaemonNavigation(page);
    await project.click();
  }
  await expect(page.getByRole('heading', { name: 'Your workflows', exact: true })).toBeVisible();
}

const esc = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const inspector = (page: Page) => page.getByRole('complementary', { name: 'Selected item', exact: true });
const taskNode = (page: Page, title: string) => page.locator('.graph-node.task').filter({ has: page.locator('strong').filter({ hasText: new RegExp(`^${esc(title)}(?: · Task \\d+)?$`) }) });
const instructions = (page: Page) => page.getByLabel('What should this task do?', { exact: true });
const apply = (page: Page) => page.locator('.drawing-actions').getByRole('button', { name: /^(Run pending plan|Apply changes)$/ });

async function chooseAgent(page: Page, id = config.model_capability!, operation = 'model.generate') {
  await page.getByRole('combobox', { name: 'Model or agent', exact: true }).selectOption(JSON.stringify([id, operation]));
  if (operation === 'process.execute') await noProjectAccess(page);
}
async function noProjectAccess(page: Page) {
  const permissions = inspector(page).locator('.task-permissions');
  if (await permissions.getAttribute('open') === null) {
    await permissions.locator(':scope > summary').focus();
    await page.keyboard.press('Enter');
  }
  await expect(permissions).toHaveAttribute('open', '');
  await page.getByRole('combobox', { name: 'Task project access', exact: true }).selectOption('none');
}
async function closeDetails(page: Page) {
  const close = page.getByRole('button', { name: 'Close details', exact: true });
  if (await close.count()) await close.click();
}
async function selectTask(page: Page, title: string) {
  await expect(page.getByRole('button', { name: '+ Add task', exact: true })).toBeEnabled();
  await closeDetails(page);
  await page.getByRole('button', { name: 'Canvas', exact: true }).click();
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await taskNode(page, title).locator('.node-content').click();
}
async function addTask(page: Page, title: string, id = config.model_capability!, operation = 'model.generate') {
  await closeDetails(page);
  await page.getByRole('button', { name: '+ Add task', exact: true }).click();
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await page.locator('.graph-node.task').filter({ has: page.locator('strong', { hasText: /^New task$/ }) }).last().locator('.node-content').click();
  await instructions(page).fill(title);
  await page.getByLabel('Task title', { exact: true }).fill(title);
  await chooseAgent(page, id, operation);
}
async function newTask(page: Page, title: string, id = config.model_capability!, operation = 'model.generate') {
  await openDaemonNavigation(page);
  await page.getByRole('button', { name: 'All workflows', exact: true }).click();
  await page.getByRole('button', { name: 'New workflow', exact: true }).click();
  await expect(page.locator('.save-state')).toHaveText('Saved');
  await page.locator('.workflow-title-button').click();
  await page.getByLabel('Workflow name', { exact: true }).fill(title);
  await page.getByLabel('Workflow name', { exact: true }).press('Enter');
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
  await addTask(page, title, id, operation);
}
async function attach(page: Page, file: string | { name: string; mimeType: string; buffer: Buffer }) {
  const selected = page.waitForEvent('filechooser');
  await inspector(page).getByRole('button', { name: 'Attach file', exact: true }).click();
  await (await selected).setFiles(file);
  await expect(inspector(page)).toContainText('A captured file');
}
async function addText(page: Page, text: string) {
  await inspector(page).getByRole('button', { name: '+ Add input', exact: true }).click();
  await page.getByLabel('Input text', { exact: true }).fill(text);
}
async function fillTask(page: Page, title: string, inputPath: string, deferred = false) {
  await newTask(page, title, config.capability, 'process.execute');
  if (deferred) {
    await inspector(page).locator('summary').filter({ hasText: /^More actions$/ }).focus();
    await page.keyboard.press('Enter');
    await expect(inspector(page).locator('details').filter({ has: page.locator('summary', { hasText: /^More actions$/ }) })).toHaveAttribute('open', '');
    await page.getByRole('button', { name: 'Wait for a required input', exact: true }).click();
  } else await attach(page, inputPath);
  await selectTask(page, title);
}
async function submit(page: Page, title: string, inputPath: string, deferred = false) {
  await fillTask(page, title, inputPath, deferred);
  if ((page.viewportSize()?.width ?? 1280) <= 760) await closeDetails(page);
  await apply(page).click();
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
}
async function waitState(page: Page, title: string, state: string) {
  await expect(taskNode(page, title).and(page.locator('.graph-node.task:not(.is-draft)')).locator('.node-state')).toContainText(state, { timeout: 45000 });
}
async function result(page: Page, title: string) {
  await waitState(page, title, 'completed');
  await selectTask(page, title);
  await page.getByRole('button', { name: 'Read result', exact: true }).click();
  return page.getByTestId('result-output');
}
async function waitMissing(page: Page, title: string) {
  await selectTask(page, title);
  await expect(inspector(page)).toContainText('Waiting for a required input');
  // The detail read can precede the workflow generation that recorded this readiness reason.
  await expect(taskNode(page, title).locator('.node-description')).toHaveText('Waiting for a required input');
}
async function supply(page: Page, inputPath: string) {
  await closeDetails(page);
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await page.locator('.graph-node.source').filter({ has: page.locator('strong', { hasText: /^Required input$/ }) }).locator('.node-content').click();
  await page.getByLabel('Replacement text', { exact: true }).fill(readFileSync(inputPath, 'utf8'));
  await page.getByRole('button', { name: 'Supply input', exact: true }).click();
}
async function selectWorkflow(page: Page, title: string) {
  await openDaemonNavigation(page);
  const row = page.locator('.daemon-row').filter({ has: page.locator('.tree-label.active') });
  const expansion = row.locator('.tree-expand');
  if (await expansion.getAttribute('aria-expanded') === 'false') await expansion.click();
  const branch = page.locator('.daemon-row:has(.tree-label.active) + .tree-children');
  await branch.locator('.tree-search input').fill(title);
  await branch.locator('.tree-search button').click();
  await branch.locator('.tree-results button').filter({ has: page.locator('span', { hasText: new RegExp(`^${esc(title)}$`) }) }).click();
  await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
}
async function daemonSettings(page: Page) {
  await openDaemonNavigation(page);
  await page.locator('.daemon-row').filter({ has: page.locator('.tree-label.active') }).getByRole('button', { name: /^Settings for / }).click();
}
async function chooseDaemon(page: Page, name: string) {
  await openDaemonNavigation(page);
  await page.locator('.daemon-row .tree-label').filter({ hasText: new RegExp(`^${esc(name)}$`) }).click();
}
async function discoverWorkflow(page: Page, title: string) {
  await openDaemonNavigation(page);
  await page.getByRole('button', { name: 'All workflows', exact: true }).click();
  await page.locator('main').getByRole('searchbox').fill(title);
  await page.locator('main').getByRole('button', { name: 'Search', exact: true }).click();
  const matching = page.locator('.workflow-list').getByRole('button').filter({ has: page.getByRole('heading', { name: title, exact: true }) });
  await expect(matching).toHaveCount(1);
  await matching.click();
}
async function createSimpleWorkflow(page: Page, title: string) {
  await newTask(page, title);
  await apply(page).click();
  await waitState(page, title, 'completed');
}
async function workflowDecision(page: Page, decision: 'wait' | 'continue', question = '') {
  await expect(page.getByRole('button', { name: '+ Add task', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Workflow', exact: true }).click();
  const submitted = page.waitForResponse((response) => response.request().method() === 'POST' && response.url().endsWith('/changes'));
  if (decision === 'wait') {
    await page.getByRole('button', { name: 'Wait for a decision', exact: true }).click();
    await page.getByLabel('What needs a decision?', { exact: true }).fill(question);
    await page.getByRole('button', { name: 'Wait for this decision', exact: true }).click();
  } else await page.getByRole('button', { name: 'Resume workflow', exact: true }).click();
  expect((await submitted).ok()).toBe(true);
  await expect(page.getByRole('region', { name: 'Workflow pause', exact: true })).toHaveCount(decision === 'wait' ? 1 : 0);
  await expect(page.getByRole('button', { name: '+ Add task', exact: true })).toBeEnabled();
}
async function connectNodes(page: Page, source: string, destination: string, outcome = false) {
  await closeDetails(page);
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  const sourceNode = page.locator('.graph-node').filter({ has: page.locator('strong').filter({ hasText: new RegExp(`^${esc(source)}$`) }) });
  await sourceNode.locator('.output-port').first().click();
  await taskNode(page, destination).locator('.input-port').first().click();
  if (outcome) {
    const connections = page.locator('.canvas-connections');
    if (await connections.getAttribute('open') === null) await connections.locator('summary').click();
    await connections.getByRole('button').filter({ hasText: `${source} → ${destination}` }).click();
    await inspector(page).getByText('When the source fails or has no result', { exact: true }).click();
    await inspector(page).getByLabel('Receive the outcome, including failure').check();
  }
}

test('project folders, shared environment and identical workflow names survive a daemon restart and fresh client', async ({ browser }, testInfo) => {
  test.setTimeout(120000);
  const context = await browser.newContext();
  const page = await context.newPage();
  const title = `Project coding ${Date.now()}`;
  const folder = `browser-code-${Date.now()}`;
  const existing = join(config.planning_project_root, 'existing.rs');
  const original = readFileSync(existing, 'utf8');
  const metadata = statSync(config.planning_project_root);
  let configured = 0;
  const configureCargo = async (current: Page) => {
    await openDaemonNavigation(current);
    await current.locator('main').getByRole('button', { name: 'Project settings', exact: true }).click();
    const select = current.getByRole('combobox', { name: 'Execution environment', exact: true });
    await expect(select).toBeEnabled();
    const options = await select.locator('option').evaluateAll((items) => items.map((item) => (item as HTMLOptionElement).value));
    const choice = options.find((value) => { const parsed = JSON.parse(value) as { capability?: string }; return parsed.capability === 'project-cargo'; });
    expect(choice).toBeTruthy();
    await select.selectOption(choice!);
    await current.getByRole('checkbox', { name: 'Allow project changes', exact: true }).check();
    await current.getByRole('button', { name: 'Save project settings', exact: true }).click();
    await expect(current.getByRole('button', { name: 'Save project settings', exact: true })).toBeEnabled();
    await current.screenshot({ path: testInfo.outputPath(`project-settings-${++configured}.png`), fullPage: true });
    await current.getByRole('button', { name: 'Back to work', exact: true }).click();
  };
  await page.goto('/'); await connect(page, config.a); await configureCargo(page);
  await newTask(page, title);
  await instructions(page).fill('Project fixture: Rust code\nCreate, edit and test a small Rust library in this project.');
  await apply(page).click();
  await expect(await result(page, title)).toContainText('Rust project created');
  await createSimpleWorkflow(page, `${title} follow-up`);
  await openDaemonNavigation(page);
  const expansion = page.locator('.daemon-row').filter({ has: page.locator('.tree-label.active') }).locator('.tree-expand');
  if (await expansion.getAttribute('aria-expanded') === 'false') await expansion.click();
  await page.locator('.daemon-tree').getByRole('button', { name: '+ Add project', exact: true }).click();
  await page.getByRole('button', { name: 'Create folder', exact: true }).click();
  await page.getByRole('button', { name: 'Browse permitted folders', exact: true }).click();
  await page.getByRole('button', { name: 'Use daemon-a', exact: true }).click();
  await page.getByLabel('New folder name', { exact: true }).fill(folder);
  await page.screenshot({ path: testInfo.outputPath('create-project-folder.png'), fullPage: true });
  await page.getByRole('button', { name: 'Create project folder', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Project settings', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Back to work', exact: true }).click();
  await configureCargo(page);
  await newTask(page, title);
  await instructions(page).fill('Project fixture: Rust code\nCreate, edit and test a small Rust library in this separate project.');
  await apply(page).click();
  await expect(await result(page, title)).toContainText('Rust project created');
  const second = join(dirname(config.planning_project_root), folder);
  for (const directory of [config.planning_project_root, second]) {
    expect(readFileSync(join(directory, 'lib.rs'), 'utf8')).toContain('a + b');
    expect(statSync(join(directory, 'target/debug/deps')).isDirectory()).toBe(true);
  }
  expect(readFileSync(existing, 'utf8')).toBe(original);
  const after = statSync(config.planning_project_root);
  expect([after.uid, after.gid, after.mode]).toEqual([metadata.uid, metadata.gid, metadata.mode]);
  await page.screenshot({ path: testInfo.outputPath('project-coding-result.png'), fullPage: true });
  await context.close();
  const root = dirname(resolve(path!));
  execFileSync(join(root, 'tools/cargo-report'), ['control', '--root', root, 'restart-a'], { timeout: 20000, stdio: 'pipe' });
  const fresh = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  try {
    const reopened = await fresh.newPage();
    await reopened.goto('/'); await connect(reopened, config.a); await discoverWorkflow(reopened, title);
    await expect(reopened.locator('.workspace-context')).toContainText('Animation project');
    await openDaemonNavigation(reopened);
    await reopened.getByRole('button', { name: 'Expand Studio A', exact: true }).click();
    await reopened.getByRole('button', { name: `Expand project ${folder}`, exact: true }).click();
    const folderBranch = reopened.locator('.tree-workflows');
    await expect(folderBranch).toContainText(title); await expect(folderBranch).not.toContainText(`${title} follow-up`);
    await folderBranch.getByRole('button').filter({ hasText: title }).click();
    await expect(await result(reopened, title)).toContainText('Rust project created');
    expect(await reopened.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await reopened.screenshot({ path: testInfo.outputPath('project-reconnected-phone.png'), fullPage: true });
  } finally { await fresh.close(); }
});

test('a real uploaded project produces discoverable input-dependent results after the browser closes', async ({ browser }, testInfo) => {
  const title = `Cargo analysis ${Date.now()}`;
  const first = await browser.newContext();
  const initial = await first.newPage();
  await initial.goto('/'); await connect(initial, config.a);
  await submit(initial, title, config.input_a);
  await waitState(initial, title, 'running');
  await first.close();
  const fresh = await browser.newContext();
  const page = await fresh.newPage();
  await page.goto('/');
  expect(await page.evaluate(() => localStorage.length)).toBe(0);
  await connect(page, config.a); await discoverWorkflow(page, title);
  const output = await result(page, title);
  for (const text of config.expect_a ?? ['dependencies']) await expect(output).toContainText(text);
  const firstResult = await output.innerText();
  await page.getByRole('button', { name: 'Hide result', exact: true }).click();
  await expect(output).toHaveCount(0); await page.keyboard.press('Enter');
  await expect(output).toHaveText(firstResult);
  await page.screenshot({ path: testInfo.outputPath('desktop-result.png'), fullPage: true });
  const varied = `${title} varied`;
  await submit(page, varied, config.input_b);
  const other = await result(page, varied);
  for (const text of config.expect_b ?? ['dependencies']) await expect(other).toContainText(text);
  expect(await other.innerText()).not.toBe(firstResult);
  await page.setViewportSize({ width: 390, height: 844 });
  await closeDetails(page);
  await page.getByRole('button', { name: 'List', exact: true }).click();
  await expect(page.locator('.canvas-list')).toContainText('completed');
  await page.locator('.list-node').filter({ hasText: varied }).click();
  await page.getByRole('button', { name: 'Read result', exact: true }).click();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('phone-result.png'), fullPage: true });
  await connect(page, config.b);
  await page.locator('main').getByRole('searchbox').fill(title); await page.locator('main').getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'No matching workflows', exact: true })).toBeVisible();
  await openSavedConnections(page); await page.getByRole('button', { name: /^Studio A/ }).click();
  await expect(page.getByRole('heading', { name: varied, level: 1, exact: true })).toBeVisible();
  const persisted = await page.evaluate(() => JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage } }));
  expect(persisted).not.toContain(config.a.token); expect(persisted).not.toContain(config.b.token);
  await openDaemonNavigation(page); await page.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await expect(page.locator('.graph-surface')).toHaveCount(0);
  await fresh.close();
});

test('a queued missing-input task can be edited and repaired through the same inspector', async ({ page }) => {
  const title = `Queued Cargo ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await submit(page, title, config.input_b, true);
  await waitMissing(page, title);
  await page.getByRole('button', { name: 'Edit task', exact: true }).click();
  await instructions(page).fill('Inspect only the captured runtime Cargo manifest and report its actual dependency names.');
  await apply(page).click();
  await waitMissing(page, title);
  await expect(inspector(page)).toContainText('Inspect only the captured runtime Cargo manifest');
  await supply(page, config.input_b); await apply(page).click();
  await expect(page.getByRole('button', { name: '+ Add task', exact: true })).toBeEnabled();
  if (await page.getByRole('button', { name: 'Refresh workflow for review', exact: true }).isVisible()) {
    await page.getByRole('button', { name: 'Refresh workflow for review', exact: true }).click(); await apply(page).click();
  }
  const output = await result(page, title);
  for (const text of config.expect_b ?? ['dependencies']) await expect(output).toContainText(text);
  await expect(page.getByRole('button', { name: 'Edit task', exact: true })).toHaveCount(0);
});

test('the right inspector preserves a long-title draft, keyboard focus and enlarged phone text', async ({ page }, testInfo) => {
  const title = `Animation research across rendering, learned motion, training data and validation ${Date.now()}`;
  const draft = 'Keep this unsaved instruction while inspecting the same queued work on a narrow screen.';
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/'); await connect(page, config.a); await submit(page, title, config.input_a, true);
  await waitMissing(page, title); await page.getByRole('button', { name: 'Edit task', exact: true }).click();
  await instructions(page).fill(draft);
  const [side, canvas] = await Promise.all([inspector(page).boundingBox(), page.locator('.graph-surface').boundingBox()]);
  expect(side!.x).toBeGreaterThan(canvas!.x + canvas!.width / 2);
  expect(side!.width).toBeLessThan(canvas!.width);
  await page.screenshot({ path: testInfo.outputPath('right-inspector-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await instructions(page).focus(); await page.keyboard.press('Escape');
  await expect(taskNode(page, title).locator('.node-content')).toBeFocused();
  await expect(instructions(page)).toHaveCount(0);
  await page.keyboard.press('Enter'); await expect(instructions(page)).toHaveValue(draft);
  await page.screenshot({ path: testInfo.outputPath('right-inspector-phone.png'), fullPage: true });
  const textSize = await instructions(page).evaluate((element) => parseFloat(getComputedStyle(element).fontSize));
  await page.evaluate(() => { document.documentElement.style.fontSize = '200%'; });
  await expect.poll(() => instructions(page).evaluate((element) => parseFloat(getComputedStyle(element).fontSize))).toBeGreaterThan(textSize * 1.9);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await expect(instructions(page)).toHaveValue(draft);
  const panelBounds = await inspector(page).boundingBox(); expect(panelBounds!.height).toBeGreaterThan(844 * .6);
  await expect(page.getByRole('button', { name: '+ Add task', exact: true })).not.toBeVisible();
  const camera = await page.locator('.graph-plane').getAttribute('style');
  await instructions(page).scrollIntoViewIfNeeded(); await instructions(page).fill(`${draft}\nA useful edit at enlarged text size.`);
  expect(await instructions(page).evaluate((element) => { const bounds = element.getBoundingClientRect(); return document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2) === element; })).toBe(true);
  await expect(page.locator('.save-state')).toHaveText('Saved');
  await page.screenshot({ path: testInfo.outputPath('right-inspector-enlarged-phone.png'), fullPage: true });
  const model = page.getByRole('combobox', { name: 'Model or agent', exact: true }); await model.scrollIntoViewIfNeeded(); await model.focus(); await expect(model).toBeFocused();
  const discard = inspector(page).getByRole('button', { name: 'Discard edit', exact: true }); await discard.scrollIntoViewIfNeeded(); await discard.focus(); await expect(discard).toBeFocused();
  await expect.poll(() => inspector(page).evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await page.getByRole('button', { name: 'Close details', exact: true }).click();
  await expect(taskNode(page, title).locator('.node-content')).toBeFocused(); await expect(page.locator('.graph-plane')).toHaveAttribute('style', camera!);
  await expect(page.getByRole('button', { name: '+ Add task', exact: true })).toBeVisible(); await expect(apply(page)).toBeVisible();
  await page.keyboard.press('Enter'); await expect(instructions(page)).toHaveValue(`${draft}\nA useful edit at enlarged text size.`);
});

test('queued cancellation permits linked correction without inventing original execution times', async ({ page }) => {
  const title = `Queued stop ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await submit(page, title, config.input_a, true);
  await waitMissing(page, title);
  await page.getByRole('button', { name: 'Request stop', exact: true }).click();
  await waitState(page, title, 'stopped'); await selectTask(page, title);
  await page.getByText('Timing & history', { exact: true }).click(); await expect(inspector(page)).toContainText('Not started');
  await page.getByRole('button', { name: 'Create correction', exact: true }).click();
  await page.getByLabel('Task title', { exact: true }).fill(`Correction ${title}`);
  await instructions(page).fill('Correct the stopped analysis using the newly supplied manifest.');
  await supply(page, config.input_a);
  const request = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith('/authoring/run'));
  await apply(page).click();
  const accepted = (await request).postDataJSON();
  expect(accepted.tasks).toHaveLength(1);
  expect(accepted.changes).toHaveLength(1);
  await result(page, `Correction ${title}`);
  await selectTask(page, title); await page.getByText('Timing & history', { exact: true }).click();
  await expect(inspector(page)).toContainText('Not started');
  await expect(taskNode(page, title)).toContainText('stopped');
});

test('an invalid supplied manifest leaves a readable failed task and blocked workflow', async ({ page }, testInfo) => {
  const title = `Invalid manifest ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await newTask(page, title, config.capability, 'process.execute');
  await attach(page, { name: 'Broken.toml', mimeType: 'text/plain', buffer: Buffer.from('[package\nname = "unfinished manifest"\n') });
  await apply(page).click(); await waitState(page, title, 'failed'); await selectTask(page, title);
  await expect(inspector(page).getByText('Failed', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Create correction', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Edit task', exact: true })).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath('failed-task-desktop.png'), fullPage: true });
  await page.getByRole('button', { name: 'Workflow', exact: true }).click();
  await expect(inspector(page).getByText('blocked', { exact: true })).toBeVisible();
});

test('one drawing atomically revises a queued task and supplies its missing input', async ({ page }, testInfo) => {
  const title = `Atomic repair ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await submit(page, title, config.input_b, true); await waitMissing(page, title);
  const requests: string[] = [];
  page.on('request', (r) => { if (r.method() === 'POST' && r.url().endsWith('/authoring/run')) requests.push(r.postData()!); });
  await page.getByRole('button', { name: 'Edit task', exact: true }).click();
  await instructions(page).fill('Read this captured manifest and report the actual runtime dependencies.');
  await supply(page, config.input_b);
  expect(requests).toHaveLength(0);
  await page.screenshot({ path: testInfo.outputPath('atomic-repair-drawing.png'), fullPage: true });
  await apply(page).click(); await result(page, title);
  expect(requests).toHaveLength(1);
  expect(JSON.parse(requests[0])).toMatchObject({ tasks: [expect.any(String)], changes: [0] });
  await expect(inspector(page)).toContainText('Read this captured manifest and report the actual runtime dependencies.');
  await page.getByText('Inputs used at start', { exact: true }).click();
  await expect(inspector(page)).toContainText(readFileSync(config.input_b, 'utf8').slice(0, 150));
});

test('a cyclic connector is refused while the unsubmitted drawing and accepted work remain', async ({ page }) => {
  const title = `Cycle base ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await createSimpleWorkflow(page, title);
  await addTask(page, 'First proposed task'); await addTask(page, 'Second proposed task');
  await connectNodes(page, 'First proposed task', 'Second proposed task');
  await connectNodes(page, 'Second proposed task', 'First proposed task');
  await apply(page).click();
  await expect(page.getByRole('region', { name: 'Tasks to run' })).toContainText(/cycle|cyclic|self/i);
  await expect(page.locator('.graph-node.task')).toHaveCount(3);
  await expect(taskNode(page, title)).toContainText('completed');
  await expect(page.locator('.graph-node.is-draft.task')).toHaveCount(2);
});

test('a lost accepted Run reply is recovered automatically by its public receipt without resubmission', async ({ page }) => {
  const title = `Lost reply ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await fillTask(page, title, config.input_a);
  let submitted = 0, receipts = 0;
  page.on('request', (request) => { if (request.method() === 'GET' && request.url().includes('/v1/workflow-requests/')) receipts++; });
  await page.route('**/v1/workflows/*/authoring/run', async (route) => { submitted++; const response = await route.fetch(); expect(response.ok()).toBe(true); await route.abort('failed'); });
  await apply(page).click(); await result(page, title);
  expect(submitted).toBe(1); expect(receipts).toBe(1);
});

test('the configured controlled model receives separate context and a captured file', async ({ page }) => {
  const title = `Controlled model ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await newTask(page, title);
  await addText(page, 'Develop a Rust-based approach to learned animation.');
  await selectTask(page, title); await attach(page, config.input_b);
  await apply(page).click(); const output = await result(page, title);
  for (const text of config.model_expect ?? ['Controlled analysis']) await expect(output).toContainText(text);
  await page.getByText('Inputs used at start', { exact: true }).click();
  await expect(inspector(page)).toContainText('Develop a Rust-based approach to learned animation.');
  await expect(inspector(page)).toContainText('Cargo.toml');
});

test('the daemon admits model-authored parallel research and correction without browser continuation', async ({ page }, testInfo) => {
  test.setTimeout(120000);
  const title = `Automatic animation ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a);
  await newTask(page, title, config.planning_capability!);
  await instructions(page).fill('Plan parallel animation research, correct the failed research prospectively and finish after the final report.');
  await addText(page, 'Develop a Rust approach to learned animation. Compare motion representation, training data and render validation in separate folders.');
  await selectTask(page, title);
  await page.getByText('Tools & autonomy', { exact: true }).click();
  await page.getByLabel('Let this model propose follow-up tasks', { exact: true }).check();
  await page.getByLabel('Let it retry refused proposals within this workflow’s limits', { exact: true }).check();
  await page.getByRole('button', { name: 'Workflow', exact: true }).click();
  let continuations = 0;
  page.on('request', (r) => { if (r.method() === 'POST' && r.url().endsWith('/changes')) continuations++; });
  await apply(page).click();
  await expect.poll(() => page.locator('.graph-node.task .node-state.running').count(), { timeout: 30000 }).toBeGreaterThanOrEqual(2);
  await page.getByRole('button', { name: 'Timeline', exact: true }).click();
  await expect.poll(() => page.getByLabel('Recorded task lifetimes').getByRole('button', { name: /, external entry recorded, no end recorded$/ }).count()).toBeGreaterThanOrEqual(2);
  await page.screenshot({ path: testInfo.outputPath('parallel-runtime-evidence.png'), fullPage: true });
  await page.getByRole('button', { name: 'Canvas', exact: true }).click();
  await result(page, 'Final animation report'); expect(continuations).toBe(0);
  await expect(page.getByTestId('result-output')).toHaveText(/^Final animation report/);
  await expect(taskNode(page, 'Research render validation')).toContainText('failed');
  await page.getByRole('button', { name: 'Timeline', exact: true }).click();
  const bars = page.getByLabel('Recorded task lifetimes');
  const interval = async (title: string) => bars.getByRole('button', { name: new RegExp(`^${title}: start recorded`) }).evaluate((e) => ({ left: parseFloat((e as HTMLElement).style.left), width: parseFloat((e as HTMLElement).style.width) }));
  const [motion, training] = await Promise.all([interval('Research motion representation'), interval('Research training data')]);
  expect(motion.left).toBeLessThan(training.left + training.width); expect(training.left).toBeLessThan(motion.left + motion.width);
  await selectTask(page, 'Animation composition'); await page.getByText('Inputs used at start', { exact: true }).click();
  await expect(inspector(page)).toContainText(/fail/i);
});

test('two workflows retain their own canvas drafts across navigation and ordinary additions', async ({ page, browser }, testInfo) => {
  const prefix = `Workspace ${Date.now()}`, a = `${prefix} A`, b = `${prefix} B`, draft = 'Summarize the next animation experiment.';
  const changes: string[] = []; let creates = 0;
  page.on('request', (r) => { if (r.method() !== 'POST') return; if (r.url().endsWith('/v1/workflows')) creates++; if (r.url().endsWith('/authoring/run')) changes.push(r.postData()!); });
  await page.goto('/'); await connect(page, config.a); await createSimpleWorkflow(page, a); await createSimpleWorkflow(page, b);
  await selectWorkflow(page, a); await addTask(page, draft);
  await expect(inspector(page).locator('textarea[required], input[required]')).toHaveCount(1);
  await expect(page.getByLabel('Batch alias', { exact: true })).toHaveCount(0);
  await selectWorkflow(page, b); await expect(instructions(page)).toHaveCount(0);
  await page.goBack(); await expect(instructions(page)).toHaveValue(draft);
  await page.goForward(); await expect(taskNode(page, b)).toBeVisible();
  await selectWorkflow(page, a); await expect(instructions(page)).toHaveValue(draft);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await instructions(page).focus(); await page.keyboard.press('Escape');
  await expect(taskNode(page, draft).locator('.node-content')).toBeFocused();
  await page.keyboard.press('Enter'); await expect(instructions(page)).toHaveValue(draft);
  await page.screenshot({ path: testInfo.outputPath('workspace-draft-phone.png'), fullPage: true });
  await closeDetails(page);
  await apply(page).click(); await waitState(page, draft, 'completed');
  expect(creates).toBe(2); expect(changes).toHaveLength(3);
  expect(JSON.parse(changes[2])).toMatchObject({ tasks: [expect.any(String)], changes: [] });
  await selectTask(page, draft); await expect(inspector(page)).toContainText(draft);
  await selectWorkflow(page, b); await expect(page.locator('.graph-node.task')).toHaveCount(1);
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain(draft);
  const fresh = await browser.newContext();
  try { const observer = await fresh.newPage(); await observer.goto('/'); await connect(observer, config.a); await observer.getByRole('searchbox').fill(prefix); await observer.getByRole('button', { name: 'Search', exact: true }).click(); await expect(observer.locator('.workflow-list').getByRole('button')).toHaveCount(2); } finally { await fresh.close(); }
});

test('ordinary additions isolate context and files while a human wait and missing input remain', async ({ page }) => {
  const title = `Isolation ${Date.now()}`, second = 'Second context task', third = 'Third context task', question = 'Keep future entries waiting for my explicit decision.';
  await page.goto('/'); await connect(page, config.a); await submit(page, title, config.input_a, true); await waitMissing(page, title);
  await workflowDecision(page, 'wait', question);
  for (const [task, text, file] of [[second, 'Only second context B.', config.input_b], [third, 'Only third context C.', config.input_a]]) {
    await addTask(page, task); await noProjectAccess(page); await addText(page, text); await selectTask(page, task); await attach(page, file); await apply(page).click();
    await page.getByRole('button', { name: 'Workflow', exact: true }).click(); await expect(inspector(page)).toContainText(question);
  }
  await expect(page.locator('.graph-node.task .node-state.queued')).toHaveCount(3);
  await workflowDecision(page, 'continue');
  for (const [task, own, other] of [[second, 'Only second context B.', 'Only third context C.'], [third, 'Only third context C.', 'Only second context B.']]) {
    await result(page, task); await page.getByText('Inputs used at start', { exact: true }).click();
    await expect(inspector(page)).toContainText(own); await expect(inspector(page)).not.toContainText(other);
  }
  await waitMissing(page, title);
});

test('supplying an input preserves the human pause and resume leaves unrelated authoring intact', async ({ page }, testInfo) => {
  const title = `Paused input ${Date.now()}`, draft = 'Keep this unfinished idea', question = 'Wait for my decision even when the manifest arrives.';
  await page.goto('/'); await connect(page, config.a); await submit(page, title, config.input_a, true); await waitMissing(page, title);
  await workflowDecision(page, 'wait', question);
  await supply(page, config.input_a); await apply(page).click();
  const pause = page.getByRole('region', { name: 'Workflow pause', exact: true });
  await expect(pause).toContainText(question);
  await expect(taskNode(page, title).locator('.node-state')).toHaveText(/queued/);
  await expect(taskNode(page, title).locator('.node-description')).toHaveText('Workflow paused');
  await selectTask(page, title); await page.getByText('Timing & history', { exact: true }).click();
  await expect(inspector(page)).toContainText('Not started');
  await addTask(page, draft);
  const position = await page.locator('.graph-plane').getAttribute('style');
  await page.screenshot({ path: testInfo.outputPath('explicit-pause-with-draft.png'), fullPage: true });
  const resumed = page.waitForRequest((request) => request.method() === 'POST' && request.url().endsWith('/changes'));
  await pause.getByRole('button', { name: 'Resume workflow', exact: true }).click();
  expect((await resumed).postDataJSON().changes).toEqual([{ kind: 'continue' }]);
  await expect(pause).toHaveCount(0);
  await expect(instructions(page)).toHaveValue(draft);
  await expect(page.locator('.graph-node.is-draft.task')).toHaveCount(1);
  await expect(page.locator('.graph-plane')).toHaveAttribute('style', position!);
  await waitState(page, title, 'completed');
});

test('a lost pause reply recovers its exact control without submitting the selected draft', async ({ page }) => {
  const title = `Pause recovery ${Date.now()}`, draft = 'Keep this idea out of the pause request';
  await page.goto('/'); await connect(page, config.a); await createSimpleWorkflow(page, title); await addTask(page, draft);
  const position = await page.locator('.graph-plane').getAttribute('style');
  const requests: { body: string; url: string }[] = [];
  await page.route('**/v1/workflows/*/changes', async (route) => {
    requests.push({ body: route.request().postData()!, url: route.request().url() });
    if (requests.length === 1) { const response = await route.fetch(); expect(response.ok()).toBe(true); await route.abort('failed'); }
    else await route.continue();
  });
  await page.getByRole('button', { name: 'Pause workflow', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Find submission receipt', exact: true })).toBeEnabled();
  const request = JSON.parse(requests[0].body);
  expect(request.changes).toEqual([{ kind: 'wait', question: 'Paused by you.' }]);
  await expect(instructions(page)).toBeDisabled();
  const receipt = page.waitForRequest((read) => read.method() === 'GET' && read.url().endsWith(`/v1/workflow-requests/${encodeURIComponent(request.request_id)}`));
  await page.getByRole('button', { name: 'Find submission receipt', exact: true }).click(); await receipt;
  await expect(page.getByRole('button', { name: 'Find submission receipt', exact: true })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Workflow pause', exact: true })).toContainText('Paused by you.');
  await expect(instructions(page)).toBeEnabled(); await expect(instructions(page)).toHaveValue(draft);
  await expect(page.locator('.graph-node.is-draft.task')).toHaveCount(1);
  await expect(page.locator('.graph-plane')).toHaveAttribute('style', position!);
  expect(requests).toHaveLength(1);
});

test('busy project work stays frozen through pause and restart then resumes once', async ({ page, browser }, testInfo) => {
  test.setTimeout(120000);
  const root = dirname(resolve(path!)), fixture = join(root, 'tools/cargo-report');
  execFileSync(fixture, ['lifecycle', '--root', root], { timeout: 30000, stdio: 'pipe' });
  const seed = JSON.parse(readFileSync(join(root, 'lifecycle-seed.json'), 'utf8')) as { workflow: string; title: string; first: string; first_title: string; second: string; second_title: string; started_at: number };
  let released = false;
  try {
    await page.goto('/'); await connect(page, config.a);
    await selectWorkflow(page, seed.title); await selectTask(page, seed.second_title);
    await expect(inspector(page)).toContainText('Project is in use');
    await expect(inspector(page)).toContainText(/Waiting before entry|Preparing/);
    await expect(inspector(page)).toContainText('External entry has not been recorded');
    await expect(page.getByRole('button', { name: 'Edit task', exact: true })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Remove unstarted task', exact: true })).toHaveCount(0);
    await expect(taskNode(page, seed.second_title)).toHaveAttribute('data-node', seed.second);
    const position = await page.locator('.graph-plane').getAttribute('style');
    await page.screenshot({ path: testInfo.outputPath('project-busy-before-entry.png'), fullPage: true });
    await page.getByRole('button', { name: 'Pause workflow', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Workflow pause', exact: true })).toBeVisible();
    await expect(inspector(page).getByRole('heading', { name: seed.second_title, exact: true })).toBeVisible();
    await expect(page.locator('.graph-plane')).toHaveAttribute('style', position!);
    execFileSync(fixture, ['lifecycle', '--root', root, '--release'], { timeout: 10000, stdio: 'pipe' }); released = true;
    await waitState(page, seed.first_title, 'completed');
    expect(readFileSync(join(root, 'daemon-a/lifecycle-project/entries'), 'utf8')).toBe('entered\n');
    await page.close();
    execFileSync(fixture, ['control', '--root', root, 'restart-a'], { timeout: 20000, stdio: 'pipe' });
    const fresh = await browser.newContext({ viewport: { width: 390, height: 844 } });
    try {
      const observer = await fresh.newPage(); await observer.goto('/'); await connect(observer, config.a);
      await selectWorkflow(observer, seed.title); await selectTask(observer, seed.second_title);
      await expect(observer.getByRole('region', { name: 'Workflow pause', exact: true })).toBeVisible();
      await expect(inspector(observer)).toContainText('Waiting before entry');
      await observer.getByText('Timing & history', { exact: true }).click();
      await expect(inspector(observer)).toContainText('No entry recorded');
      await observer.screenshot({ path: testInfo.outputPath('paused-recovered-task-narrow.png'), fullPage: true });
      await observer.getByRole('button', { name: 'Resume workflow', exact: true }).click();
      await waitState(observer, seed.second_title, 'completed');
      const response = await observer.request.get(`${config.a.url}/v1/workflows/${encodeURIComponent(seed.workflow)}/tasks/${encodeURIComponent(seed.second)}`, { headers: { authorization: `Bearer ${config.a.token}` } });
      expect(response.ok()).toBe(true);
      const task = (await response.json()).value;
      expect(task.started_at).toBe(seed.started_at); expect(task.frozen.entered).toBe(true);
      expect(readFileSync(join(root, 'daemon-a/lifecycle-project/entries'), 'utf8')).toBe('entered\nentered\n');
    } finally { await fresh.close(); }
  } finally {
    if (!released) execFileSync(fixture, ['lifecycle', '--root', root, '--release'], { timeout: 10000, stdio: 'pipe' });
  }
});

for (const recovery of ['receipt', 'replay'] as const) {
  test(`an unconfirmed Run keeps its exact workflow through navigation and ${recovery} recovery`, async ({ page }) => {
    // Two real workflow runs, daemon switching and exact recovery have separate bounded waits.
    test.setTimeout(120000);
    const a = `Recover A ${recovery} ${Date.now()}`, b = `Recover B ${recovery} ${Date.now()}`, draft = `Retain this ${recovery} submission on A.`;
    await page.goto('/'); await connect(page, config.a); await createSimpleWorkflow(page, a); await createSimpleWorkflow(page, b); await selectWorkflow(page, a); await addTask(page, draft);
    const requests: { url: string; body: string }[] = []; let release!: () => void;
    const released = new Promise<void>((resolve) => { release = resolve; });
    await page.route('**/v1/workflows/*/authoring/run', async (route) => {
      requests.push({ url: route.request().url(), body: route.request().postData()! });
      if (requests.length === 1) { if (recovery === 'receipt') { const actual = await route.fetch(); expect(actual.ok()).toBe(true); } await released; await route.abort('failed'); } else await route.continue();
    });
    await apply(page).click(); await expect.poll(() => requests.length).toBe(1); await expect(instructions(page)).toBeDisabled();
    await selectWorkflow(page, b); release(); await connect(page, config.b);
    await expect(page.getByRole('button', { name: 'Find submission receipt', exact: true })).toHaveCount(0);
    await chooseDaemon(page, 'Studio A'); await selectWorkflow(page, a);
    if (recovery === 'replay') {
      await expect(instructions(page)).toHaveValue(draft); await expect(instructions(page)).toBeDisabled();
      await page.getByRole('button', { name: 'Replay exact submission', exact: true }).click();
    }
    await expect(page.getByRole('button', { name: 'Find submission receipt', exact: true })).toHaveCount(0);
    await expect(page.locator('.drawing-actions')).toHaveCount(0); await expect(page.locator('.graph-node.is-draft.task')).toHaveCount(0);
    await waitState(page, draft, 'completed'); expect(requests).toHaveLength(recovery === 'receipt' ? 1 : 2);
    if (recovery === 'replay') expect(requests[1]).toEqual(requests[0]);
    await selectWorkflow(page, b); await expect(page.locator('.graph-node.task')).toHaveCount(1);
  });
}

test('a stale addition retains its drawing and needs a fresh decision without clearing a human wait', async ({ page, browser }) => {
  test.setTimeout(120000);
  const title = `Stale workspace ${Date.now()}`, draft = 'Review this addition against the newer workflow.';
  await page.goto('/'); await connect(page, config.a); await createSimpleWorkflow(page, title); await addTask(page, draft);
  await expect(page.locator('.save-state')).toHaveText('Saved');
  let release!: () => void;
  const released = new Promise<void>((resolve) => { release = resolve; });
  let reviews = 0;
  await page.route('**/v1/workflows/*/authoring/review', async (route) => {
    const response = await route.fetch(); expect(response.ok()).toBe(true);
    if (++reviews === 1) await released;
    await route.fulfill({ response });
  });
  const requests: string[] = [];
  page.on('request', (request) => { if (request.method() === 'POST' && request.url().endsWith('/authoring/run')) requests.push(request.postData()!); });
  const other = await browser.newContext();
  try {
    const editor = await other.newPage(); await editor.goto('/'); await connect(editor, config.a); await discoverWorkflow(editor, title);
    // Hold only the competing decision, not setup of an entire authenticated client.
    await apply(page).click(); await expect.poll(() => reviews).toBe(1);
    await workflowDecision(editor, 'wait', 'A separate client requested this decision.');
    const refused = page.waitForResponse((response) => response.request().method() === 'POST' && response.url().endsWith('/authoring/run'));
    release(); expect((await refused).status()).toBe(409);
    await expect(page.locator('.canvas-message[role="alert"]')).toContainText('stale workflow generation');
    await expect(instructions(page)).toHaveValue(draft); expect(requests).toHaveLength(1);
    await page.getByRole('button', { name: 'Refresh workflow for review', exact: true }).click(); expect(requests).toHaveLength(1);
    await apply(page).click(); await expect(page.locator('.graph-node.task:not(.is-draft)')).toHaveCount(2);
    const [old, fresh] = requests.map((request) => JSON.parse(request));
    expect(fresh.request_id).not.toBe(old.request_id); expect(fresh.expected_generation).toBeGreaterThan(old.expected_generation);
    expect(fresh.tasks).toEqual(old.tasks); expect(fresh.changes).toEqual(old.changes); expect(fresh.use_current).toEqual(old.use_current);
    await expect(page.getByRole('region', { name: 'Workflow pause', exact: true })).toContainText('A separate client requested this decision.');
    await expect(taskNode(page, draft)).toContainText('queued');
  } finally { release(); await other.close(); }
});

test('an actual daemon authorization refusal disposes the inaccessible drawing', async ({ page }) => {
  const title = `Authority ${Date.now()}`, draft = 'This private draft must disappear with this session.';
  await page.goto('/'); await connect(page, config.a); await createSimpleWorkflow(page, title); await addTask(page, draft);
  await expect(page.locator('.save-state')).toHaveText('Saved');
  let runs = 0; page.on('request', (request) => { if (request.method() === 'POST' && request.url().endsWith('/authoring/run')) runs++; });
  await page.route('**/v1/workflows/*/authoring/review', (route) => route.continue({ headers: { ...route.request().headers(), authorization: 'Bearer invalid-after-session-established' } }));
  await apply(page).click(); await expect(page.getByRole('alert')).toContainText('Authenticate again');
  await expect(instructions(page)).toHaveCount(0); await expect(page.locator('.graph-surface')).toHaveCount(0); await expect(page.getByText(draft, { exact: true })).toHaveCount(0);
  expect(runs).toBe(0);
  await page.unroute('**/v1/workflows/*/authoring/review'); await openSavedConnections(page); await page.getByRole('button', { name: /^Studio A/ }).click();
  await page.getByLabel('Bearer credential').fill(config.a.token); await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await discoverWorkflow(page, title); await expect(page.locator('.save-state')).toHaveText('Saved');
  await expect(page.locator('.graph-node.task:not(.is-draft)')).toHaveCount(1);
  await expect(page.locator('.graph-node.task.is-draft')).toHaveCount(1);
  await selectTask(page, draft); await expect(instructions(page)).toHaveValue(draft);
  expect(runs).toBe(0);
});

test('a retained named process result feeds a model through a visible connector after reconnect', async ({ page, browser }, testInfo) => {
  // Both actual executions and the fresh authenticated client must finish in this journey.
  test.setTimeout(120000);
  const title = `File to process ${Date.now()}`, consumer = 'Compose the named process evidence';
  await page.goto('/'); await connect(page, config.a); await submit(page, title, config.input_b); await result(page, title);
  await addTask(page, consumer); await connectNodes(page, title, consumer);
  const request = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith('/authoring/run'));
  await apply(page).click(); const body = (await request).postDataJSON();
  expect(body.tasks).toHaveLength(1); expect(body.changes).toHaveLength(0);
  await result(page, consumer); await page.getByText('Inputs used at start', { exact: true }).click();
  await expect(inspector(page)).toContainText('Captured immutable input');
  await page.screenshot({ path: testInfo.outputPath('file-process-model-canvas.png'), fullPage: true });
  const other = await browser.newContext();
  try { const fresh = await other.newPage(); await fresh.goto('/'); await connect(fresh, config.a); await discoverWorkflow(fresh, title); await expect(fresh.locator('.graph-node.task')).toHaveCount(2); await expect(fresh.locator('.graph-node.source')).toHaveCount(1); await fresh.getByText('Connections (2)', { exact: true }).click(); await expect(fresh.locator('.canvas-connections')).toContainText(consumer); } finally { await other.close(); }
});

test('several connected draft tasks are submitted atomically and independent research starts concurrently', async ({ page }, testInfo) => {
  const title = `Atomic drawing ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await newTask(page, title, config.capability, 'process.execute'); await attach(page, config.input_a);
  await addTask(page, 'Research animation', config.capability, 'process.execute'); await attach(page, config.input_b);
  await addTask(page, 'Research validation', config.capability, 'process.execute'); await attach(page, config.input_a);
  await addTask(page, 'Compose animation');
  await connectNodes(page, title, 'Compose animation'); await connectNodes(page, 'Research animation', 'Compose animation'); await connectNodes(page, 'Research validation', 'Compose animation');
  let requests = 0; page.on('request', (r) => { if (r.method() === 'POST' && r.url().endsWith('/authoring/run')) requests++; });
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith('/authoring/run'));
  expect(requests).toBe(0);
  await closeDetails(page); await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await page.screenshot({ path: testInfo.outputPath('four-task-drawing-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 }); await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await page.screenshot({ path: testInfo.outputPath('four-task-drawing-phone.png'), fullPage: true });
  await page.setViewportSize({ width: 1280, height: 720 });
  await apply(page).click(); const body = (await sent).postDataJSON();
  await expect.poll(() => page.locator('.graph-node.task .node-state.running').count()).toBeGreaterThanOrEqual(2);
  expect(body.tasks).toHaveLength(4); expect(requests).toBe(1);
  await result(page, 'Compose animation'); await expect(page.locator('.graph-node.task')).toHaveCount(4);
  const captured = inspector(page).locator('details').filter({ has: page.getByText('Inputs used at start', { exact: true }) });
  await captured.locator('summary').click(); await expect(captured.locator('h3')).toHaveCount(3);
  await expect(captured).toContainText('Captured immutable input');
});

test('keyboard canvas movement and camera navigation remain local to each workflow', async ({ page }) => {
  // Two real workflow runs and three discovery round trips share this scenario.
  // Keep each action's deadline; allow their bounded successful steps to finish.
  test.setTimeout(120000);
  const a = `Camera A ${Date.now()}`, b = `Camera B ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await createSimpleWorkflow(page, a); await createSimpleWorkflow(page, b); await selectWorkflow(page, a);
  let mutations = 0; page.on('request', (r) => { if (r.method() === 'POST') mutations++; });
  const surface = page.getByRole('region', { name: 'Workflow canvas', exact: true }), plane = page.locator('.graph-plane');
  await surface.focus(); await page.keyboard.press('ArrowLeft'); await page.keyboard.press('+');
  const camera = await plane.getAttribute('style');
  const node = taskNode(page, a), before = await node.getAttribute('style');
  await node.getByRole('button', { name: `Move ${a}`, exact: true }).focus(); await page.keyboard.press('ArrowDown');
  expect(await node.getAttribute('style')).not.toBe(before);
  await selectWorkflow(page, b); expect(await plane.getAttribute('style')).not.toBe(camera);
  await selectWorkflow(page, a); expect(await plane.getAttribute('style')).toBe(camera); expect(mutations).toBe(0);
});

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
  await expect(page.getByRole('heading', { name: 'Your projects', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add project', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'No accessible workflows', exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('empty-discovery-phone.png'), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({ path: testInfo.outputPath('empty-discovery-desktop.png'), fullPage: true });
  await daemonSettings(page);
  await expect(page.getByLabel('Daemon name')).toHaveValue('Studio B');
  await page.getByText('Connection details', { exact: true }).click();
  await expect(page.getByText(config.b.identity, { exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('daemon-settings-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('daemon-settings-phone.png'), fullPage: true });
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

test('keyboard access and identity refusal are visible on a phone layout', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(({ endpoint }) => {
    window.localStorage.setItem('milkdrift.connections.v1', JSON.stringify([{ id: 'old-profile', label: 'Old studio', endpoint, daemonId: 'not-the-configured-installation' }]));
  }, { endpoint: config.a.url });
  await page.goto('/');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Skip to workspace' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Workspace navigation', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Workspace navigation', exact: true })).toHaveAttribute('aria-expanded', 'true');
  await page.getByRole('button', { name: /^Old studio/ }).click();
  await page.getByLabel('Bearer credential').fill(config.a.token);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('different daemon');
  await expect(page.getByRole('heading', { name: 'Your workflows' })).toHaveCount(0);
  expect(await page.evaluate(() => JSON.parse(window.localStorage.getItem('milkdrift.connections.v1')!)[0].daemonId)).toBe('not-the-configured-installation');
  await expect(page.getByLabel('Bearer credential')).toHaveValue('');
  await openSavedConnections(page);
  await page.getByRole('button', { name: 'Forget Old studio connection' }).click();
  await connect(page, config.a);
  await openSavedConnections(page);
  await expect(page.getByRole('button', { name: /^Studio A/ })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});

test('a browser at an unconfigured origin receives a useful connection refusal', async ({ page }) => {
  await page.goto(`http://127.0.0.1:${Number(process.env.MILKDRIFT_BROWSER_PORT ?? '4173') + 1}`);
  await page.getByRole('button', { name: 'Connect a daemon', exact: true }).click();
  await page.getByLabel('Daemon URL').fill(config.a.url);
  await page.getByLabel('Bearer credential').fill(config.a.token);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('allowed browser origin');
  await expect(page.getByLabel('Bearer credential')).toHaveValue('');
  await expect(page.getByRole('heading', { name: 'Your workflows', exact: true })).toHaveCount(0);
});

test('independent clients share a recovered rename and distinguish duplicate names by daemon address', async ({ browser }, testInfo) => {
  test.setTimeout(120000);
  const controller = await browser.newContext();
  const observer = await browser.newContext();
  const first = await controller.newPage();
  const second = await observer.newPage();
  try {
  await first.goto('/');
  await second.goto('/');
  await connect(first, config.a);
  await connect(second, config.a);
  await second.getByRole('searchbox').fill(`No workflow matches this ${Date.now()}`);
  await second.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(second.getByRole('heading', { name: 'No matching workflows' })).toBeVisible();
  await connect(first, config.b);
  await openSavedConnections(first);
  await first.getByRole('button', { name: /^Studio A/ }).click();
  await daemonSettings(first);
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
    await daemonSettings(first);
    await expect(first.getByRole('button', { name: 'Find rename receipt' })).toBeVisible();
    await openSavedConnections(second);
    await expect(second.getByRole('button', { name: /^Studio B/ })).toBeVisible();
    await first.getByRole('button', { name: 'Find rename receipt' }).click();
    await expect(first.getByRole('status')).toContainText('Name saved on this daemon');
    await expect(first.getByRole('button', { name: 'Save daemon name', exact: true })).toBeEnabled();
    expect(submitted).toBe(1);
    const duplicates = first.getByRole('button', { name: /^Studio B/ });
    await openSavedConnections(first);
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
    await expect(second.getByRole('button', { name: /^Studio B/ })).toContainText('Reconnect');
    await second.getByRole('button', { name: /^Studio B/ }).click();
    await second.getByLabel('Bearer credential').fill(config.a.token);
    await second.getByRole('button', { name: 'Connect', exact: true }).click();
    await expect(second.getByRole('heading', { name: 'Your workflows', exact: true })).toBeVisible();
    await expect(second.locator('.workspace-context')).toContainText('Animation project');
    await openSavedConnections(second);
    await expect(second.getByRole('button', { name: /^Studio B/ }).locator('.status-dot.connected')).toHaveCount(1);
    const fresh = await browser.newContext();
    try {
      const freshPage = await fresh.newPage();
      await freshPage.goto('/');
      await connect(freshPage, config.a);
      await openSavedConnections(freshPage);
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
    await openDaemonNavigation(first);
    await first.getByRole('button', { name: 'New workflow', exact: true }).click();
    await expect(first.getByRole('heading', { name: 'Untitled workflow', exact: true })).toBeVisible();
    expect(await first.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await first.screenshot({ path: testInfo.outputPath('long-daemon-name-composer-phone.png'), fullPage: true });
    await first.getByRole('button', { name: '+ Add task', exact: true }).click();
    await first.getByRole('button', { name: 'New task · Draft', exact: true }).click();
    await first.getByRole('button', { name: 'Discard task', exact: true }).click();
    await openDaemonNavigation(first);
    await daemonSettings(first);
    await expect(first.getByLabel('Daemon name')).toHaveValue(longName);
  } finally {
    await first.unroute('**/v1/daemon/rename');
    // Leave the shared review fixture with its seeded human-readable name.
    if (!await first.getByLabel('Daemon name').count()) {
      await openDaemonNavigation(first);
      await daemonSettings(first);
    }
    if (await first.getByRole('button', { name: 'Find rename receipt' }).count()) {
      await first.getByRole('button', { name: 'Find rename receipt' }).click();
    }
    await first.getByLabel('Daemon name').fill('Studio A');
    await first.getByRole('button', { name: 'Save daemon name', exact: true }).click();
    await openSavedConnections(first);
    await expect(first.getByRole('button', { name: /^Studio A/ })).toBeVisible();
  }
  } finally { await controller.close(); await observer.close(); }
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
  await expect(page.getByRole('heading', { name: 'Your projects' })).toBeVisible();
});

test('touch alternatives distinguish duplicate task titles and preserve assignment while opening Models and agents', async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  try {
    const page = await context.newPage(); await page.goto('/'); await connect(page, config.a);
    await newTask(page, 'Repeated task'); await addTask(page, 'Repeated task');
    const selected = page.getByRole('combobox', { name: 'Model or agent', exact: true });
    await expect(selected).toHaveValue(JSON.stringify([config.model_capability, 'model.generate']));
    await page.getByRole('button', { name: '+ Add model or agent', exact: true }).tap();
    await expect(page.getByRole('heading', { name: 'Models & agents', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Back to canvas', exact: true }).tap();
    await expect(instructions(page)).toHaveValue('Repeated task');
    await expect(selected).toHaveValue(JSON.stringify([config.model_capability, 'model.generate']));
    await page.getByRole('button', { name: 'Close details', exact: true }).tap();
    await page.getByRole('button', { name: 'Fit', exact: true }).tap();
    const repeated = taskNode(page, 'Repeated task');
    await repeated.first().locator('.output-port').first().tap();
    await repeated.nth(1).locator('.input-port').first().tap();
    await expect(page.getByRole('combobox', { name: 'Destination task', exact: true })).toHaveCount(0);
    await expect(page.getByText('Connections (1)', { exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  } finally { await context.close(); }
});


test('dragging directly between real ports saves the wire without an execution or destination form', async ({ page }, testInfo) => {
  await page.goto('/'); await connect(page, config.a); await newTask(page, 'Use dragged context');
  await page.getByRole('button', { name: '+ Text', exact: true }).click();
  await page.getByLabel('Input text', { exact: true }).fill('Research context to connect');
  await closeDetails(page); await page.getByRole('button', { name: 'Fit', exact: true }).click();
  const source = await page.getByRole('button', { name: 'Connect Output from Research context to connect', exact: true }).boundingBox();
  const destination = await taskNode(page, 'Use dragged context').locator('.input-port').first().boundingBox();
  expect(source).toBeTruthy(); expect(destination).toBeTruthy();
  let submitted = 0; page.on('request', (request) => { if (request.method() === 'POST') submitted++; });
  await page.mouse.move(source!.x + source!.width / 2, source!.y + source!.height / 2);
  await page.mouse.down(); await page.mouse.move(destination!.x + destination!.width / 2, destination!.y + destination!.height / 2, { steps: 12 }); await page.mouse.up();
  await expect(page.getByRole('combobox', { name: 'Destination task', exact: true })).toHaveCount(0);
  await expect(page.getByText('Connections (1)', { exact: true })).toBeVisible();
  await expect(page.locator('.save-state')).toHaveText('Saved'); expect(submitted).toBe(0);
  await page.screenshot({ path: testInfo.outputPath('direct-port-connection.png'), fullPage: true });
});

test('a saved plan larger than one Run stays intact and can be deliberately narrowed', async ({ page }) => {
  test.setTimeout(120000);
  await page.goto('/'); await connect(page, config.a); await newTask(page, 'First retained draft');
  for (let index = 1; index < 33; index++) await addTask(page, `Retained draft ${index + 1}`);
  await expect(page.locator('.graph-node.is-draft.task')).toHaveCount(33);
  await expect(page.locator('.save-state')).toHaveText('Saved');
  await apply(page).click(); await expect(page.getByRole('region', { name: 'Tasks to run' })).toContainText(/32|capacity|limit/i);
  await expect(page.locator('.graph-node.is-draft.task')).toHaveCount(33);
  await page.getByRole('button', { name: 'Close run scope', exact: true }).click();
  await selectTask(page, 'Retained draft 33'); await page.getByRole('button', { name: 'Discard task', exact: true }).click();
  await expect(page.locator('.graph-node.is-draft.task')).toHaveCount(32); await expect(page.locator('.save-state')).toHaveText('Saved');
});

test('a saved task with a mismatched reviewed model is refused until its model is explicitly reviewed', async ({ page }) => {
  const title = `Review changed model ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await newTask(page, title); await addText(page, 'Context must remain attached');
  await expect(page.locator('.save-state')).toHaveText('Saved');
  const workflowId = await page.evaluate(() => window.history.state.milkdrift.workflow as string);
  const endpoint = `${config.a.url}/v1/workflows/${encodeURIComponent(workflowId)}/authoring`;
  const headers = { authorization: `Bearer ${config.a.token}` };
  const saved = (await (await page.request.get(endpoint, { headers })).json()).value;
  saved.document.tasks[0].contract_generation++;
  const update = await page.request.put(endpoint, { headers, data: { expected_generation: saved.generation, document: saved.document } }); expect(update.ok()).toBe(true);
  await selectWorkflow(page, title);
  // A new client observation loads the real saved candidate; no successful daemon response is mocked.
  await page.reload(); await openSavedConnections(page); await page.getByRole('button', { name: /^Studio A/ }).click();
  await page.getByLabel('Bearer credential').fill(config.a.token); await page.getByRole('button', { name: 'Connect', exact: true }).click(); await discoverWorkflow(page, title);
  let runs = 0; page.on('request', (request) => { if (request.method() === 'POST' && request.url().endsWith('/authoring/run')) runs++; });
  await apply(page).click(); await expect(page.getByRole('region', { name: 'Tasks to run' })).toContainText(/changed|generation|review/i);
  expect(runs).toBe(0); await expect(page.locator('.graph-node.is-draft.task')).toHaveCount(1); await expect(page.locator('.graph-node.source')).toHaveCount(1);
  await page.getByRole('button', { name: 'Close run scope', exact: true }).click(); await selectTask(page, title);
  await page.getByRole('button', { name: 'Use reviewed model', exact: true }).click(); await apply(page).click(); await result(page, title); expect(runs).toBe(1);
});

test('a failed producer blocks its result consumer until the user deliberately chooses the retained outcome', async ({ page }) => {
  const title = `Failure policy ${Date.now()}`, consumer = 'Review the failed analysis';
  await page.goto('/'); await connect(page, config.a); await newTask(page, title, config.capability, 'process.execute');
  await attach(page, { name: 'Broken.toml', mimeType: 'text/plain', buffer: Buffer.from('[package\nname = "broken manifest"\n') });
  await addTask(page, consumer); await connectNodes(page, title, consumer);
  await apply(page).click(); await waitState(page, title, 'failed');
  await expect(taskNode(page, consumer).locator('.node-state')).toContainText('queued');
  await selectTask(page, consumer); await page.getByRole('button', { name: 'Edit task', exact: true }).click();
  await inspector(page).locator('.inspector-connections').getByRole('button').filter({ hasText: title }).click();
  await page.getByRole('button', { name: 'Use the outcome even if it fails', exact: true }).click();
  const submission = page.waitForRequest((request) => request.method() === 'POST' && request.url().endsWith('/authoring/run'));
  await apply(page).click();
  const body = (await submission).postDataJSON();
  expect(body.tasks).toHaveLength(1); expect(body.changes).toHaveLength(0);
  await result(page, consumer);
  await expect(taskNode(page, title).locator('.node-state')).toContainText('failed');
  await expect(inspector(page)).toContainText('Outcome, including failure');
});


test('replacing a shared input visibly affects future starts while an earlier task keeps its frozen context', async ({ page }) => {
  const title = `Shared context ${Date.now()}`, future = 'Future shared-context consumer';
  await page.goto('/'); await connect(page, config.a); await newTask(page, title);
  await addText(page, 'Shared context to supply');
  await page.getByLabel('Supply this input later', { exact: true }).check();
  await addTask(page, future);
  await connectNodes(page, 'Shared context to supply', future);
  await selectTask(page, future);
  await addText(page, 'Hold the future task');
  await page.getByLabel('Supply this input later', { exact: true }).check();
  await apply(page).click(); await waitState(page, title, 'queued');
  // Queued is already true at creation. Wait for the scheduler's recorded input reasons so
  // the first edit reviews that generation, rather than racing those two durable observations.
  await expect(taskNode(page, title)).toContainText('Waiting for a required input');
  await expect(taskNode(page, future)).toContainText('Waiting for a required input');
  await expect(page.getByRole('button', { name: '+ Add task', exact: true })).toBeEnabled();
  await closeDetails(page); await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await page.locator('.graph-node.source .node-content').first().click();
  await expect(inspector(page)).toContainText(title); await expect(inspector(page)).toContainText(future);
  await page.getByLabel('Replacement text', { exact: true }).fill('Original shared context');
  await page.getByRole('button', { name: 'Supply input', exact: true }).click(); await apply(page).click();
  await waitState(page, title, 'completed'); await expect(taskNode(page, future).locator('.node-state')).toContainText('queued');
  await expect(page.getByRole('button', { name: '+ Add task', exact: true })).toBeEnabled();
  await closeDetails(page); await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await page.locator('.graph-node.source .node-content').filter({ hasText: 'Original shared context' }).click();
  await expect(inspector(page).getByText(/^Affected future tasks:/)).toHaveText(`Affected future tasks: ${future}.`);
  await page.getByLabel('Replacement text', { exact: true }).fill('Replacement shared context');
  await page.getByRole('button', { name: 'Supply input', exact: true }).click(); await apply(page).click();
  await selectTask(page, title); await page.getByText('Inputs used at start', { exact: true }).click();
  const frozen = inspector(page).locator('details').filter({ has: page.getByText('Inputs used at start', { exact: true }) });
  await expect(frozen).toContainText('Original shared context'); await expect(frozen).not.toContainText('Replacement shared context');
  await closeDetails(page); await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await page.locator('.graph-node.source').filter({ has: page.locator('strong', { hasText: /^Hold the future task$/ }) }).locator('.node-content').click();
  await page.getByLabel('Replacement text', { exact: true }).fill('The later task can now start');
  await page.getByRole('button', { name: 'Supply input', exact: true }).click(); await apply(page).click();
  await result(page, future); await page.getByText('Inputs used at start', { exact: true }).click();
  const later = inspector(page).locator('details').filter({ has: page.getByText('Inputs used at start', { exact: true }) });
  await expect(later).toContainText('Replacement shared context'); await expect(later).not.toContainText('Original shared context');
});

test('a start racing an edit refuses the drawing while retaining instructions and actual execution history', async ({ page, browser }) => {
  test.setTimeout(120000);
  const title = `Start race ${Date.now()}`, changed = 'Keep this prospective edit after another client starts the task.';
  await page.goto('/'); await connect(page, config.a); await submit(page, title, config.input_b, true); await waitMissing(page, title);
  const taskId = await taskNode(page, title).getAttribute('data-node');
  const workflowId = await page.evaluate(() => window.history.state.milkdrift.workflow as string);
  await page.getByRole('button', { name: 'Edit task', exact: true }).click(); await instructions(page).fill(changed);
  await expect(page.locator('.save-state')).toHaveText('Saved');
  const other = await browser.newContext();
  try {
    const second = await other.newPage(); await second.goto('/'); await connect(second, config.a); await discoverWorkflow(second, title);
    await expect(second.locator('.save-state')).toHaveText('Saved'); await expect(second.locator('.graph-node.task.is-draft')).toHaveCount(1);
    await closeDetails(second); await second.getByRole('button', { name: 'Fit', exact: true }).click();
    await second.locator('.graph-node.source .node-content').click();
    await second.getByLabel('Replacement text', { exact: true }).fill(readFileSync(config.input_b, 'utf8'));
    await second.getByRole('button', { name: 'Supply input', exact: true }).click();
    await second.getByRole('button', { name: 'Choose tasks to run', exact: true }).click();
    const scope = second.getByRole('region', { name: 'Tasks to run', exact: true });
    await scope.getByRole('checkbox', { name: `${title} · Apply candidate edit`, exact: true }).uncheck();
    await scope.getByRole('button', { name: 'Review this plan', exact: true }).click();
    const submitted = second.waitForRequest((request) => request.method() === 'POST' && request.url().endsWith('/authoring/run'));
    await scope.getByRole('button', { name: 'Run reviewed plan', exact: true }).click();
    expect((await submitted).postDataJSON()).toMatchObject({ tasks: [], changes: [0] });
    await expect(taskNode(second, title).locator('.node-state')).toContainText('running', { timeout: 45000 });
    // The input submission updates the shared document. Reopen its current saved candidate
    // before reviewing the task's separate start conflict.
    await openDaemonNavigation(page); await page.getByRole('button', { name: 'All workflows', exact: true }).click();
    await discoverWorkflow(page, title); await expect(page.locator('.save-state')).toHaveText('Saved');
    await selectTask(page, title); await expect(instructions(page)).toHaveValue(changed);
    let runs = 0; page.on('request', (request) => { if (request.method() === 'POST' && request.url().endsWith('/authoring/run')) runs++; });
    const reviewed = page.waitForResponse((response) => response.request().method() === 'POST' && response.url().endsWith('/authoring/review'));
    await apply(page).click(); const review = await (await reviewed).json();
    expect(review.value.issues).toEqual(expect.arrayContaining([expect.objectContaining({ code: 'start_conflict' })]));
    await expect(page.getByRole('region', { name: 'Tasks to run', exact: true })).toContainText('This task changed or started');
    await expect(instructions(page)).toHaveValue(changed); expect(runs).toBe(0);
    await expect(taskNode(second, title).locator('.node-state')).toContainText('completed', { timeout: 45000 });
    const actual = await second.request.get(`${config.a.url}/v1/workflows/${encodeURIComponent(workflowId)}/tasks/${encodeURIComponent(taskId!)}`, { headers: { authorization: `Bearer ${config.a.token}`, 'x-milkdrift-daemon': config.a.identity } });
    expect(actual.ok()).toBe(true); const executed = (await actual.json()).value;
    expect(executed.definition.instructions).toBe(title); expect(executed.version).toBe(1);
    expect(executed.started_at).not.toBeNull(); expect(executed.ended_at).not.toBeNull(); expect(executed.frozen.entered).toBe(true);
    await second.getByRole('button', { name: 'Timeline', exact: true }).click();
    await expect(second.getByRole('region', { name: 'Recorded task lifetimes', exact: true }).locator('.execution-bar')).toHaveAttribute('aria-label', /external entry recorded/);
    await expect(page.locator('.graph-node.task')).toHaveCount(1); await expect(instructions(page)).toHaveValue(changed);
  } finally { await other.close(); }
});


test('blank placement, unfinished wires and a usual model preserve an unrelated idea across a scoped Run and reconnect', async ({ page, browser }, testInfo) => {
  test.setTimeout(120000);
  const title = `Saved canvas journey ${Date.now()}`, first = 'Research the useful question', second = 'Summarize the connected evidence';
  await page.goto('/'); await connect(page, config.a); await page.getByRole('button', { name: 'New workflow', exact: true }).click();
  await expect(page.locator('.save-state')).toHaveText('Saved'); await page.locator('.workflow-title-button').click(); await page.getByLabel('Workflow name', { exact: true }).fill(title); await page.getByLabel('Workflow name', { exact: true }).press('Enter');
  const camera = await page.locator('.graph-plane').getAttribute('style');
  for (let index = 0; index < 3; index++) await page.getByRole('button', { name: '+ Add task', exact: true }).click();
  await expect(inspector(page)).toHaveCount(0); await expect(page.locator('.graph-plane')).toHaveAttribute('style', camera!);
  await expect(page.locator('.graph-node.task')).toHaveCount(3); await expect(page.locator('.graph-node.task').first()).toContainText('Choose a model');
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  const cards = page.locator('.graph-node.task'); await cards.nth(0).locator('.output-port').click(); await cards.nth(1).locator('.input-port').click();
  await expect(page.locator('.unfinished-wire')).toHaveCount(1); await expect(inspector(page)).toHaveCount(0);
  await page.getByRole('button', { name: 'Usual model…', exact: true }).click(); await page.getByRole('combobox', { name: 'Model for this canvas', exact: true }).selectOption(JSON.stringify([config.model_capability, 'model.generate'])); await page.getByRole('button', { name: 'Use for these 3 tasks and new tasks', exact: true }).click();
  await cards.nth(0).locator('.node-content').click(); await instructions(page).fill(first); await closeDetails(page); await cards.nth(1).locator('.node-content').click(); await instructions(page).fill(second);
  await expect(page.locator('.unfinished-wire')).toHaveCount(0); await expect(page.locator('.save-state')).toHaveText('Saved');
  await page.getByRole('button', { name: 'Choose tasks to run', exact: true }).click(); const plan = page.getByRole('region', { name: 'Tasks to run' });
  await plan.getByRole('checkbox', { name: first, exact: true }).uncheck(); await plan.getByRole('checkbox', { name: 'New task', exact: true }).uncheck();
  await plan.getByRole('button', { name: 'Review this plan', exact: true }).click(); await expect(plan.getByRole('checkbox', { name: first, exact: true })).toBeChecked(); await expect(plan.getByRole('checkbox', { name: 'New task', exact: true })).not.toBeChecked();
  await page.screenshot({ path: testInfo.outputPath('saved-plan-reviewed-predecessor.png'), fullPage: true });
  await plan.getByRole('button', { name: 'Run reviewed plan', exact: true }).click(); await waitState(page, second, 'completed');
  await expect(page.locator('.graph-node.is-draft.task')).toHaveCount(1); await expect(page.locator('.graph-node.is-draft.task')).toContainText('New task');
  const fresh = await browser.newContext({ viewport: { width: 1200, height: 900 } });
  try { const observer = await fresh.newPage(); await observer.goto('/'); await connect(observer, config.a); await discoverWorkflow(observer, title); await expect(observer.locator('.graph-node.task')).toHaveCount(3); await expect(observer.locator('.graph-node.is-draft.task')).toHaveCount(1); await observer.screenshot({ path: testInfo.outputPath('saved-canvas-reopened.png'), fullPage: true }); } finally { await fresh.close(); }
});

test('a lost checkpoint reply is confirmed, and a later concurrent canvas edit requires an explicit choice', async ({ page }, testInfo) => {
  const title = `Concurrent saved canvas ${Date.now()}`;
  await page.goto('/'); await connect(page, config.a); await newTask(page, title); await expect(page.locator('.save-state')).toHaveText('Saved');
  await page.setViewportSize({ width: 390, height: 844 });
  let saves = 0, recoveredReads = 0;
  page.on('request', (request) => { if (request.method() === 'GET' && request.url().endsWith('/authoring')) recoveredReads++; });
  await page.route('**/v1/workflows/*/authoring', async (route) => { if (route.request().method() !== 'PUT') { await route.continue(); return; } saves++; const actual = await route.fetch(); expect(actual.ok()).toBe(true); await route.abort('failed'); });
  await instructions(page).fill('Keep the first saved edit'); await expect(page.locator('.save-state')).toHaveText('Saved'); expect(saves).toBe(1); expect(recoveredReads).toBe(1);
  await page.unroute('**/v1/workflows/*/authoring');
  const workflowId = await page.evaluate(() => window.history.state.milkdrift.workflow as string), endpoint = `${config.a.url}/v1/workflows/${encodeURIComponent(workflowId)}/authoring`, headers = { authorization: `Bearer ${config.a.token}` };
  const current = (await (await page.request.get(endpoint, { headers })).json()).value; current.document.tasks[0].definition.instructions = 'A separately saved decision';
  expect((await page.request.put(endpoint, { headers, data: { expected_generation: current.generation, document: current.document } })).ok()).toBe(true);
  await instructions(page).fill('Keep my competing candidate'); await expect(page.getByText('The saved canvas changed elsewhere. Your candidate is preserved.', { exact: true })).toBeVisible(); await expect(instructions(page)).toHaveValue('Keep my competing candidate');
  await expect(inspector(page)).toBeVisible();
  const comparison = page.getByText('Compare saved and my canvas', { exact: true });
  expect(await comparison.evaluate((element) => { const bounds = element.getBoundingClientRect(); return element.contains(document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)); })).toBe(true);
  await page.getByText('Compare saved and my canvas', { exact: true }).click(); await expect(page.getByText('Saved elsewhere', { exact: true })).toBeVisible(); await page.screenshot({ path: testInfo.outputPath('saved-canvas-conflict.png'), fullPage: true });
  const useSaved = page.getByRole('button', { name: 'Use saved canvas', exact: true }); await useSaved.scrollIntoViewIfNeeded();
  expect(await useSaved.evaluate((element) => { const bounds = element.getBoundingClientRect(); return element.contains(document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)); })).toBe(true);
  await page.getByRole('button', { name: 'Use saved canvas', exact: true }).click(); await expect(instructions(page)).toHaveValue('A separately saved decision'); await expect(page.locator('.save-state')).toHaveText('Saved');
});

for (const decision of ['Use current task', 'Apply these changes'] as const) {
  test(`a queued predecessor candidate requires an explicit ${decision} decision before its consumer runs`, async ({ page }) => {
    test.setTimeout(120000);
    const title = `Predecessor ${decision} ${Date.now()}`, child = `Consumer ${decision}`, revised = `Prospective instructions for ${decision}`;
    await page.goto('/'); await connect(page, config.a); await submit(page, title, config.input_b, true); await waitMissing(page, title);
    const parentId = await taskNode(page, title).getAttribute('data-node'), workflowId = await page.evaluate(() => window.history.state.milkdrift.workflow as string);
    await page.getByRole('button', { name: 'Edit task', exact: true }).click(); await instructions(page).fill(revised);
    await addTask(page, child); await connectNodes(page, title, child); await expect(page.locator('.save-state')).toHaveText('Saved');
    await page.getByRole('button', { name: 'Choose tasks to run', exact: true }).click(); const plan = page.getByRole('region', { name: 'Tasks to run', exact: true });
    await plan.getByRole('checkbox', { name: `${title} · Apply candidate edit`, exact: true }).uncheck();
    await plan.getByRole('button', { name: 'Review this plan', exact: true }).click();
    await expect(plan.getByRole('button', { name: 'Use current task', exact: true })).toBeVisible(); await expect(plan.getByRole('button', { name: 'Apply these changes', exact: true })).toBeVisible();
    await plan.getByRole('button', { name: decision, exact: true }).click();
    await expect(plan.getByRole('button', { name: 'Run reviewed plan', exact: true })).toBeVisible();
    const sent = page.waitForRequest((request) => request.method() === 'POST' && request.url().endsWith('/authoring/run'));
    await plan.getByRole('button', { name: 'Run reviewed plan', exact: true }).click(); const request = (await sent).postDataJSON();
    expect(request.tasks).toHaveLength(decision === 'Use current task' ? 1 : 2); expect(request.use_current).toEqual(decision === 'Use current task' ? [parentId] : []);
    await expect(taskNode(page, child).and(page.locator(':not(.is-draft)'))).toHaveCount(1);
    const actual = await page.request.get(`${config.a.url}/v1/workflows/${encodeURIComponent(workflowId)}/tasks/${encodeURIComponent(parentId!)}`, { headers: { authorization: `Bearer ${config.a.token}` } }); expect(actual.ok()).toBe(true);
    const parent = (await actual.json()).value; expect(parent.started_at).toBeNull(); expect(parent.version).toBe(decision === 'Use current task' ? 1 : 2); expect(parent.definition.instructions).toBe(decision === 'Use current task' ? title : revised);
    await expect(taskNode(page, title)).toHaveClass(decision === 'Use current task' ? /is-draft/ : /^(?!.*is-draft)/);
    if (decision === 'Use current task') { await selectTask(page, title); await expect(instructions(page)).toHaveValue(revised); }
  });
}
