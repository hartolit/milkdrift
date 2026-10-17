import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import { tick } from 'svelte';
import WorkflowView from './WorkflowView.svelte';
import type { TaskReadDto, WorkflowReadDto } from '../lib/protocol.generated';
import { DaemonClient, DaemonError } from '../lib/transport';

afterEach(cleanup);

function task(overrides: Partial<TaskReadDto> = {}): TaskReadDto {
  return {
    id: 'task-a', version: 1, definition: {
      actions: null, tools: [],
      title: 'Research', description: 'Compare the supplied project', instructions: 'Inspect the manifest',
      capability: { id: 'report', operation: 'process.execute' }, inputs: {}, dependencies: [], outputs: {}, required: true,
      corrective_of: null, paths: [],
    },
    state: 'queued', reason: 'Missing required input: brief', started_at: null, ended_at: null,
    stop_requested: false, stop_request_id: 'stop-task-a', frozen: null, result: null, result_restricted: false, origin: null, ...overrides,
  };
}

async function renderWorkflow(tasks: TaskReadDto[], overrides: Partial<WorkflowReadDto> = {}, onpage: (cursor?: string) => Promise<void> = vi.fn(async () => {})) {
  const workflow: WorkflowReadDto = {
    wait_question: null, admitted_tasks: tasks.length, admitted_model_batches: 0, rejected_responses: 0, active_execution_ms: 0, admissions: [],
    id: 'workflow-a', title: 'Animation', description: 'Project research', generation: 1, state: 'waiting',
    created_at: 1000, updated_at: 1000, task_count: tasks.length, next_task_cursor: null,
    tasks: tasks.map(({ id, version, definition, state, reason, started_at, ended_at, stop_requested, stop_request_id, result }) => ({ id, version, title: definition.title, description: definition.description, dependencies: definition.dependencies, state, reason, started_at, ended_at, stop_requested, stop_request_id, has_result: result !== null })),
    limits: { tasks: 256, concurrent_tasks: 4, model_batches: 32, rejected_responses: 3, corrective_failures: 3, active_execution_ms: 3600000, tool_calls_per_task: 16 },
    ...overrides,
  };
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  vi.spyOn(client, 'request').mockImplementation(async (path) => tasks.find((task) => path.endsWith(`/tasks/${task.id}`)));
  render(WorkflowView, { workflow, client, onrefresh: vi.fn(async () => {}), onpage, onback: vi.fn(), onerror: vi.fn() });
  await screen.findByRole('heading', { name: tasks[0].definition.title, level: 2 });
  return client;
}

it('shows a queued reason without fabricating a start and keeps list actions available', async () => {
  await renderWorkflow([task()]);
  expect(screen.getAllByText('Not started')).toHaveLength(2);
  expect(screen.getByRole('button', { name: 'Edit unstarted task' })).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'List' }));
  expect(screen.getAllByText('Missing required input: brief').length).toBeGreaterThan(0);
  expect(screen.getByRole('button', { name: 'Edit unstarted task' })).toBeTruthy();
});

it('keeps cancellation requests distinct from stopped evidence and exposes expired results', async () => {
  const client = await renderWorkflow([task({ state: 'running', started_at: 2000, stop_requested: true, reason: null })]);
  vi.spyOn(client, 'request').mockRejectedValue(new DaemonError('expired', 'This result has expired.', 410));
  expect(screen.queryByRole('button', { name: 'Edit unstarted task' })).toBeNull();
  expect((screen.getByRole('button', { name: 'Request stop' }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getAllByText('Running · stop requested')).toHaveLength(2);
  await fireEvent.click(screen.getByRole('button', { name: 'Read result' }));
  expect(screen.getByRole('alert').textContent).toContain('This result has expired.');
  expect(screen.queryByTestId('result-output')).toBeNull();
});

it('does not show a late result under a different selected task', async () => {
  const first = task();
  const second = task({ id: 'task-b', definition: { ...first.definition, title: 'Composition' } });
  const client = await renderWorkflow([first, second]);
  let finish!: (value: unknown) => void;
  vi.spyOn(client, 'request').mockImplementation((path) => path.endsWith('/result') ? new Promise((resolve) => { finish = resolve; }) : Promise.resolve(second));
  await fireEvent.click(screen.getByRole('button', { name: 'Read result' }));
  await fireEvent.click(screen.getByRole('button', { name: /Composition/ }));
  finish({ text: 'Private result for the first task', digest: 'digest-a', artifacts: [], input_digests: {} });
  await tick();
  expect(screen.queryByText('Private result for the first task')).toBeNull();
});

it('explains a settled outcome without text instead of showing a blank result', async () => {
  await renderWorkflow([task({ state: 'failed', started_at: 2000, ended_at: 3000, reason: 'Output publication failed', result: { text: '', text_unavailable: false, text_truncated: false, digest: 'empty-digest', artifacts: [], input_digests: {} } })]);
  expect(screen.getByText('No text output was retained for this outcome. Inspect the task state and reason above.')).toBeTruthy();
  expect(screen.queryByTestId('result-output')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Download result' })).toBeNull();
});

it('shows explicit input restrictions and unavailable text while retaining lifecycle and artifact facts', async () => {
  await renderWorkflow([task({ state: 'completed', started_at: 2000, ended_at: 3000, reason: null,
    frozen: { capability_generation: 1, actor_id: 'actor-a', grant_id: 'grant-a', grant_revision: 1, entered: true, inputs: { file: { kind: 'text_file', restricted: true, name: null, text: null, digest: null, artifact_id: null, text_truncated: false } } },
    result: { text: '', text_unavailable: true, text_truncated: false, digest: 'exact-byte-digest', artifacts: ['artifact-a'], input_digests: {} },
  })]);
  expect(screen.getByText('This input’s contents, filename and provenance are restricted by the current grant.')).toBeTruthy();
  expect(screen.getByText('The retained output contains bytes that cannot be displayed as UTF-8 text. Retrieve the individual immutable artifacts listed below.')).toBeTruthy();
  expect(screen.getByText('artifact-a')).toBeTruthy();
  expect(screen.queryByText('No text output was retained for this outcome. Inspect the task state and reason above.')).toBeNull();
});

it('holds an unconfirmed edit unchanged until exact replay', async () => {
  const client = await renderWorkflow([task()]);
  const request = vi.spyOn(client, 'request').mockRejectedValueOnce(new Error('Reply was lost')).mockResolvedValueOnce({});
  request.mockClear();
  await fireEvent.click(screen.getByRole('button', { name: 'Edit unstarted task' }));
  await fireEvent.input(screen.getByLabelText('Instructions'), { target: { value: 'The accepted future draft' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Save future task' }).closest('form')!);
  expect((screen.getByRole('button', { name: 'Request stop' }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByLabelText('Instructions').closest('fieldset')?.disabled).toBe(true);
  await fireEvent.click(screen.getByRole('button', { name: 'Replay exact change' }));
  expect(request.mock.calls[1]).toEqual(request.mock.calls[0]);
});

it('retains supplied input text when the daemon refuses a stale generation', async () => {
  const definition = task().definition;
  definition.inputs = { brief: { kind: 'supplied', name: 'brief', optional: false } };
  const client = await renderWorkflow([task({ definition })]);
  vi.spyOn(client, 'request').mockRejectedValue(new DaemonError('conflict', 'stale workflow generation', 409));
  await fireEvent.input(screen.getByLabelText('Input text'), { target: { value: 'A retained future input' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Supply input' }).closest('form')!);
  expect(screen.getByRole('alert').textContent).toContain('stale workflow generation');
  expect((screen.getByLabelText('Input text') as HTMLTextAreaElement).value).toBe('A retained future input');
  expect((screen.getByRole('button', { name: 'Supply input' }) as HTMLButtonElement).disabled).toBe(false);
});

it('labels bounded previews and downloads complete text through the authenticated endpoint', async () => {
  const client = await renderWorkflow([task({ state: 'completed', result: { text: 'Preview only', text_unavailable: false, text_truncated: true, digest: 'complete-digest', artifacts: ['artifact-a'], input_digests: {} } })]);
  const download = vi.spyOn(client, 'download').mockRejectedValue(new DaemonError('unavailable', 'Complete text exceeds download limit; use retained artifacts.', 413));
  expect(screen.getByText(/Showing a bounded text preview/)).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Download result' }));
  expect(download).toHaveBeenCalledWith('/v1/workflows/workflow-a/tasks/task-a/result/content');
  expect(screen.getByRole('alert').textContent).toContain('Complete text exceeds download limit');
});

it('makes incomplete task pages explicit and requests the daemon continuation', async () => {
  const onpage = vi.fn<(cursor?: string) => Promise<void>>().mockResolvedValue(undefined);
  await renderWorkflow([task()], { task_count: 51, next_task_cursor: 'bound-daemon-cursor' }, onpage);
  expect(screen.getByText(/Showing 1 tasks from this page of 51/)).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Next task page' }));
  expect(onpage).toHaveBeenCalledWith('bound-daemon-cursor');
  await fireEvent.click(screen.getByRole('button', { name: 'First task page' }));
  expect(onpage).toHaveBeenLastCalledWith();
});

it('distinguishes restricted output from an absent retained result', async () => {
  await renderWorkflow([task({ state: 'completed', result_restricted: true })]);
  expect(screen.getByText(/Result contents are restricted by the current grant/)).toBeTruthy();
  expect(screen.queryByText(/No retained result is present/)).toBeNull();
  expect((screen.getByRole('button', { name: 'Read result' }) as HTMLButtonElement).disabled).toBe(true);
});

it('shows cumulative limits, pending decisions and the accepted cause of follow-up work', async () => {
  await renderWorkflow([task({ origin: { task_id: 'planner', status: 'accepted', response_digest: 'response-identity', reason: null, receipt_key: 'batch-key', aliases: { research: 'task-a' } } })], {
    wait_question: 'Review the rejected correction.', admitted_tasks: 8, admitted_model_batches: 4,
    active_execution_ms: 2345, rejected_responses: 2,
    admissions: [{ task_id: 'other-source-on-this-page', status: 'rejected', response_digest: 'other-response', reason: 'Proposed path is outside the grant.', receipt_key: null, aliases: {} }],
  });
  expect(screen.getByRole('status').textContent).toContain('Review the rejected correction.');
  expect(screen.getByText('8 / 256')).toBeTruthy();
  expect(screen.getByText('4 / 32')).toBeTruthy();
  expect(screen.getByText('Follow-up work admitted from task planner, response response-identity.')).toBeTruthy();
  expect(screen.getByText('Proposed path is outside the grant.')).toBeTruthy();
});
