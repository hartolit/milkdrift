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
      title: 'Research', description: 'Compare the supplied project', instructions: 'Inspect the manifest',
      capability: { id: 'report', operation: 'process.execute' }, inputs: {}, dependencies: [], required: true,
      corrective_of: null, paths: [],
    },
    state: 'queued', reason: 'Missing required input: brief', started_at: null, ended_at: null,
    stop_requested: false, stop_request_id: 'stop-task-a', frozen: null, result: null, ...overrides,
  };
}

function renderWorkflow(tasks: TaskReadDto[]) {
  const workflow: WorkflowReadDto = {
    id: 'workflow-a', title: 'Animation', description: 'Project research', generation: 1, state: 'waiting',
    created_at: 1000, updated_at: 1000, tasks,
    limits: { tasks: 256, concurrent_tasks: 4, model_batches: 32, rejected_responses: 3, corrective_failures: 3, active_execution_ms: 3600000, tool_calls_per_task: 16 },
  };
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  render(WorkflowView, { workflow, client, onrefresh: vi.fn(async () => {}), onback: vi.fn(), onerror: vi.fn() });
  return client;
}

it('shows a queued reason without fabricating a start and keeps list actions available', async () => {
  renderWorkflow([task()]);
  expect(screen.getAllByText('Not started')).toHaveLength(2);
  expect(screen.getByRole('button', { name: 'Edit unstarted task' })).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'List' }));
  expect(screen.getAllByText('Missing required input: brief').length).toBeGreaterThan(0);
  expect(screen.getByRole('button', { name: 'Edit unstarted task' })).toBeTruthy();
});

it('keeps cancellation requests distinct from stopped evidence and exposes expired results', async () => {
  const client = renderWorkflow([task({ state: 'running', started_at: 2000, stop_requested: true, reason: null })]);
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
  const client = renderWorkflow([first, second]);
  let finish!: (value: unknown) => void;
  vi.spyOn(client, 'request').mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  await fireEvent.click(screen.getByRole('button', { name: 'Read result' }));
  await fireEvent.click(screen.getByRole('button', { name: /Composition/ }));
  finish({ text: 'Private result for the first task', digest: 'digest-a', artifacts: [], input_digests: {} });
  await tick();
  expect(screen.queryByText('Private result for the first task')).toBeNull();
});

it('explains a settled outcome without text instead of showing a blank result', () => {
  renderWorkflow([task({ state: 'failed', started_at: 2000, ended_at: 3000, reason: 'Output publication failed', result: { text: '', digest: 'empty-digest', artifacts: [], input_digests: {} } })]);
  expect(screen.getByText('No text output was retained for this outcome. Inspect the task state and reason above.')).toBeTruthy();
  expect(screen.queryByTestId('result-output')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Download result' })).toBeNull();
});
