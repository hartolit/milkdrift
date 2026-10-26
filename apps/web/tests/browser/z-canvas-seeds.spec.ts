import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import type { TaskReadDto, WorkflowReadDto } from '../../src/lib/protocol.generated';

interface Daemon { url: string; identity: string; token: string }
interface Fixture { a: Daemon; a_limited?: Daemon; canvas_cases?: boolean }
interface Seeds {
  offpage: { workflow: string; title: string; producer: string; consumer: string };
  historical: { workflow: string; title: string; original: string; consumer: string; compatible: string; wrong_media: string; oversized: string };
  proposal: { workflow: string; title: string; task: string; wait_question: string };
  denied: { workflow: string; title: string; task: string; artifact_id: string; private_text: string };
}

const configPath = process.env.MILKDRIFT_BROWSER_CONFIG;
if (!configPath) throw new Error('Set MILKDRIFT_BROWSER_CONFIG to the private Rust fixture browser.json.');
const root = dirname(resolve(configPath));
const config = JSON.parse(readFileSync(configPath, 'utf8')) as Fixture;
let seeds: Seeds;

test.describe('public API canvas review cases', () => {
  test.beforeAll(() => {
    test.setTimeout(120000);
    if (!config.canvas_cases) throw new Error('The complete browser suite requires a fresh Rust fixture started with serve --canvas-cases.');
    const path = resolve(root, 'canvas-seeds.json');
    if (!existsSync(path)) execFileSync(resolve(root, 'tools/cargo-report'), ['seed', '--root', root], { timeout: 110000, stdio: 'pipe' });
    seeds = JSON.parse(readFileSync(path, 'utf8')) as Seeds;
  });

  const panel = (page: Page) => page.getByRole('complementary', { name: 'Selected item', exact: true });
  const apply = (page: Page) => page.locator('.drawing-actions').getByRole('button', { name: /^(Run pending plan|Apply changes)$/ });

  async function read<T>(page: Page, path: string): Promise<T> {
    const response = await page.request.get(`${config.a.url}${path}`, { headers: { authorization: `Bearer ${config.a.token}`, 'x-milkdrift-daemon': config.a.identity } });
    expect(response.ok()).toBe(true);
    return ((await response.json()) as { value: T }).value;
  }
  const task = (page: Page, workflow: string, id: string) => read<TaskReadDto>(page, `/v1/workflows/${encodeURIComponent(workflow)}/tasks/${encodeURIComponent(id)}`);
  const workflow = (page: Page, id: string) => read<WorkflowReadDto>(page, `/v1/workflows/${encodeURIComponent(id)}`);

  async function open(page: Page, title: string, daemon = config.a) {
    await page.goto('/');
    await page.getByRole('button', { name: 'Add daemon connection', exact: true }).click();
    await page.getByLabel('Daemon URL').fill(daemon.url);
    await page.getByLabel('Bearer credential').fill(daemon.token);
    await page.getByRole('button', { name: 'Connect', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Your projects', exact: true })).toBeVisible();
    await page.getByRole('searchbox').fill(title);
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    await page.locator('.workflow-list').getByRole('button').filter({ has: page.getByRole('heading', { name: title, exact: true }) }).click();
    await expect(page.getByRole('heading', { name: title, level: 1, exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'List', exact: true }).click();
  }

  async function select(page: Page, title: string) {
    await page.getByRole('button', { name: 'List', exact: true }).click();
    await page.locator('.list-node').filter({ has: page.getByText(title, { exact: true }) }).click();
    await expect(panel(page)).toBeVisible();
  }

  async function stageCorrection(page: Page, correction: string) {
    await select(page, 'Historical failed report');
    await panel(page).getByRole('button', { name: 'Use correction for future work', exact: true }).click();
    await page.getByRole('combobox', { name: 'Successful correction', exact: true }).selectOption(correction);
    await page.getByRole('checkbox', { name: 'Future report consumer', exact: true }).check();
    await page.getByRole('button', { name: 'Use selected correction', exact: true }).click();
  }

  test('a historical published output rejects incompatible corrections atomically and redirects future work', async ({ page }, testInfo) => {
    test.setTimeout(120000);
    const seed = seeds.historical;
    await open(page, seed.title);
    const original = await task(page, seed.workflow, seed.original);
    const consumer = await task(page, seed.workflow, seed.consumer);
    expect(original.state).toBe('failed');
    expect(original.definition.outputs.report).toBeUndefined();
    expect(original.result?.outputs.report.available).toBe(true);
    for (const [label, correction] of [['wrong media', seed.wrong_media], ['oversized evidence', seed.oversized]]) {
      await test.step(`refuse ${label} without changing the workflow`, async () => {
        const before = await workflow(page, seed.workflow);
        await stageCorrection(page, correction);
        const rejected = page.waitForResponse((response) => response.url().endsWith(`/workflows/${seed.workflow}/authoring/run`) && response.request().method() === 'POST');
        await apply(page).click();
        expect((await rejected).status()).toBe(400);
        await expect(page.getByRole('alert')).toBeVisible();
        expect((await workflow(page, seed.workflow)).generation).toBe(before.generation);
        expect((await task(page, seed.workflow, seed.consumer)).definition).toEqual(consumer.definition);
        await page.locator('.drawing-review summary').click();
        await page.locator('.drawing-review').getByRole('button', { name: 'Undo', exact: true }).click();
      });
    }
    await stageCorrection(page, seed.compatible);
    await apply(page).click();
    await expect.poll(async () => (await task(page, seed.workflow, seed.consumer)).state, { timeout: 45000 }).toBe('completed');
    const after = await task(page, seed.workflow, seed.consumer);
    const corrected = await task(page, seed.workflow, seed.compatible);
    expect(after.definition.inputs.report).toMatchObject({ kind: 'task_output', task: seed.compatible, output: 'report' });
    expect(after.frozen?.inputs.report.artifact_id).toBe(corrected.result?.outputs.report.artifact_id);
    const history = await task(page, seed.workflow, seed.original);
    expect(history.state).toBe('failed');
    expect(history.definition).toEqual(original.definition);
    expect(history.result).toEqual(original.result);
    expect(history.frozen).toEqual(original.frozen);
    await page.screenshot({ path: testInfo.outputPath('historical-correction.png'), fullPage: true });
  });

  test('repair and proposal dismissal share one submission without resuming a human wait', async ({ page }, testInfo) => {
    const seed = seeds.proposal;
    await open(page, seed.title);
    const before = await workflow(page, seed.workflow);
    expect(before.wait_question).toBe(seed.wait_question);
    await page.getByRole('button', { name: '+ Add task', exact: true }).click();
    await select(page, 'New task');
    await page.getByLabel('What should this task do?', { exact: true }).fill('Reviewed repair awaiting the separate budget decision');
    await page.getByRole('combobox', { name: 'Model or agent', exact: true }).selectOption(JSON.stringify(['controlled-model', 'model.generate']));
    await select(page, 'Rejected follow-up proposal');
    await page.getByRole('button', { name: 'Dismiss reviewed proposal', exact: true }).click();
    const submitted = page.waitForRequest((request) => request.url().endsWith(`/workflows/${seed.workflow}/authoring/run`) && request.method() === 'POST');
    await apply(page).click();
    const body = (await submitted).postDataJSON() as { tasks: string[]; changes: number[] };
    expect(body.tasks).toHaveLength(1);
    expect(body.changes).toEqual([0]);
    await expect.poll(async () => (await workflow(page, seed.workflow)).task_count).toBe(before.task_count + 1);
    const after = await workflow(page, seed.workflow);
    expect(after.wait_question).toBe(seed.wait_question);
    expect(after.admissions.find((admission) => admission.task_id === seed.task)?.status).toBe('dismissed');
    const repair = after.tasks.find((item) => item.id !== seed.task)!;
    const queued = await task(page, seed.workflow, repair.id);
    expect(queued.state).toBe('queued');
    expect(queued.frozen).toBeNull();
    await page.getByRole('button', { name: 'Workflow', exact: true }).click();
    await expect(panel(page)).toContainText(seed.wait_question);
    await page.screenshot({ path: testInfo.outputPath('proposal-dismissal-wait.png'), fullPage: true });
  });

  test('a partial graph edit preserves its unseen producer and survives clearing the detail cache', async ({ page }, testInfo) => {
    const seed = seeds.offpage;
    await open(page, seed.title);
    const before = await task(page, seed.workflow, seed.consumer);
    const producerBefore = await task(page, seed.workflow, seed.producer);
    expect(producerBefore.state).toBe('queued');
    expect(producerBefore.reason).toBe('missing supplied input review-hold');
    expect(producerBefore.frozen).toBeNull();
    expect(before.state).toBe('queued');
    expect(before.frozen).toBeNull();
    await page.getByRole('button', { name: 'Next task page', exact: true }).click();
    await expect(page.locator('.graph-boundary')).toContainText('Showing 10 of 60 tasks');
    await expect(page.locator('.list-node').filter({ hasText: 'Task outside this page' })).toBeVisible();
    await select(page, 'Paged consumer');
    await panel(page).getByRole('button', { name: 'Edit task', exact: true }).click();
    await panel(page).locator('.inspector-connections').getByRole('button').filter({ hasText: 'review-hold' }).click();
    await panel(page).getByRole('button', { name: 'Remove connection', exact: true }).click();
    await page.getByRole('button', { name: 'Workflow', exact: true }).click();
    await page.getByRole('button', { name: 'Clear cached details', exact: true }).click();
    await expect(apply(page)).toBeVisible();
    await expect(page.locator('.save-state')).toHaveText('Saved');
    await select(page, 'Paged consumer');
    await expect(page.getByLabel('What should this task do?', { exact: true })).toHaveValue(before.definition.instructions);
    await apply(page).click();
    await expect.poll(async () => (await task(page, seed.workflow, seed.consumer)).version).toBe(before.version + 1);
    const after = await task(page, seed.workflow, seed.consumer);
    expect(after.definition.inputs.hold).toBeUndefined();
    expect(after.definition.inputs.result).toEqual(before.definition.inputs.result);
    expect(after.definition.dependencies).toEqual(before.definition.dependencies);
    expect(after.frozen).toBeNull();
    const producerAfter = await task(page, seed.workflow, seed.producer);
    expect(producerAfter.definition).toEqual(producerBefore.definition);
    expect(producerAfter.definition.inputs.hold).toMatchObject({ kind: 'supplied', name: 'review-hold' });
    expect(producerAfter.frozen).toBeNull();
    await page.screenshot({ path: testInfo.outputPath('off-page-preserved-binding.png'), fullPage: true });
  });

  test('a valid scoped actor cannot reuse a restricted source or select an ungranted project', async ({ page }, testInfo) => {
    const limited = config.a_limited;
    if (!limited) throw new Error('Start a fresh --canvas-cases fixture with the scoped canvas actor.');
    const seed = seeds.denied;
    await open(page, seed.title, limited);
    const before = await workflow(page, seed.workflow);
    const headers = { authorization: `Bearer ${limited.token}`, 'x-milkdrift-daemon': limited.identity };
    const observed = await page.request.get(`${limited.url}/v1/workflows/${seed.workflow}`, { headers });
    expect(observed.ok()).toBe(true);
    const observedText = await observed.text();
    expect(observedText).not.toContain(seed.private_text);
    expect(observedText).not.toContain(seed.artifact_id);
    expect(JSON.parse(observedText).value.supplied_inputs['private-context']).toEqual({ availability: 'restricted', value: null });
    const denied = await page.request.get(`${limited.url}/v1/artifacts/${encodeURIComponent(seed.artifact_id)}`, { headers });
    expect(denied.status()).toBe(404);
    await expect(page.locator('.canvas-list')).toContainText('Access restricted');
    await expect(page.locator('body')).not.toContainText(seed.private_text);

    let submissions = 0;
    page.on('request', (request) => { if (request.method() === 'POST') submissions++; });
    await page.getByRole('button', { name: '+ Add task', exact: true }).click();
    await expect(page.getByRole('alert')).toContainText('project before adding a task');
    await expect(page.getByRole('combobox', { name: 'Task project access', exact: true })).toHaveCount(0);
    await expect(page.getByRole('navigation', { name: 'Daemons, projects and workflows', exact: true })).not.toContainText('Animation project');
    await page.locator('.canvas-list article').filter({ has: page.locator('.list-node').filter({ hasText: 'Access restricted' }) }).getByRole('button', { name: 'Connect to…', exact: true }).click();
    await page.getByRole('combobox', { name: 'Destination task', exact: true }).selectOption({ label: 'Awaiting authorized context' });
    await panel(page).getByRole('button', { name: 'Connect', exact: true }).click();
    await expect(page.getByRole('alert')).toContainText('restricted or expired');
    expect(submissions).toBe(0);
    const after = await workflow(page, seed.workflow);
    expect(after.generation).toBe(before.generation);
    expect(after.task_count).toBe(before.task_count);
    expect((await task(page, seed.workflow, seed.task)).frozen).toBeNull();
    await page.screenshot({ path: testInfo.outputPath('scoped-source-refusal.png'), fullPage: true });
  });
});
