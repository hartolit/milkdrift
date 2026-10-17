import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import FutureBatchEditor from './FutureBatchEditor.svelte';
import type { ChangeWorkflowRequest, TaskReadDto, WorkflowReadDto } from '../lib/protocol.generated';
import { DaemonClient, DaemonError } from '../lib/transport';

afterEach(cleanup);

function workflow(): WorkflowReadDto {
  return {
    wait_question: null, admitted_tasks: 3, admitted_model_batches: 1, rejected_responses: 0, active_execution_ms: 4, admissions: [],
    id: 'work', title: 'Research', description: '', generation: 7, state: 'waiting', created_at: 1, updated_at: 5,
    limits: { tasks: 256, concurrent_tasks: 4, model_batches: 32, rejected_responses: 3, corrective_failures: 3, active_execution_ms: 3600000, tool_calls_per_task: 16 },
    task_count: 3, next_task_cursor: null,
    tasks: [
      { id: 'original', version: 1, title: 'Failed research', description: '', dependencies: [], state: 'failed', reason: 'Failed', started_at: 1, ended_at: 2, stop_requested: false, stop_request_id: 'stop-original', has_result: true },
      { id: 'fixed', version: 1, title: 'Successful correction', description: '', dependencies: [], state: 'completed', reason: null, started_at: 3, ended_at: 4, stop_requested: false, stop_request_id: 'stop-fixed', has_result: true },
      { id: 'consumer', version: 3, title: 'Composition', description: '', dependencies: ['original'], state: 'queued', reason: 'Required producer failed', started_at: null, ended_at: null, stop_requested: false, stop_request_id: 'stop-consumer', has_result: false },
    ],
  };
}

function setup(initialTask: TaskReadDto | null = null) {
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  const request = vi.spyOn(client, 'request').mockResolvedValue({});
  const refresh = vi.fn(async () => {});
  const close = vi.fn();
  const rendered = render(FutureBatchEditor, { workflow: workflow(), client, capabilities: [{ id: 'model', operation: 'model.generate' }], initialTask, onrefresh: refresh, onclose: close, onerror: vi.fn() });
  return { client, request, refresh, close, rendered };
}

async function stage() {
  await fireEvent.submit(screen.getByRole('button', { name: 'Stage change' }).closest('form')!);
}

async function preview() {
  await fireEvent.click(screen.getByRole('button', { name: 'Preview complete batch' }));
  return JSON.parse(screen.getByTestId('batch-preview').textContent!) as ChangeWorkflowRequest;
}

it('stages complete input contracts and previews exact aliases without creating any task', async () => {
  const { request } = setup();
  await fireEvent.input(screen.getByLabelText('Batch alias'), { target: { value: 'compose' } });
  await fireEvent.input(screen.getByLabelText('Task title'), { target: { value: 'Compose research' } });
  await fireEvent.input(screen.getByLabelText('Instructions'), { target: { value: 'Compare successful notes with the failed outcome.' } });
  await fireEvent.input(screen.getByLabelText('Input binding name'), { target: { value: 'notes' } });
  await fireEvent.change(screen.getByLabelText('Input policy'), { target: { value: 'task_output' } });
  await fireEvent.input(screen.getByLabelText('Producer task identity or batch alias'), { target: { value: 'research-a' } });
  await fireEvent.input(screen.getByLabelText('Producer output name'), { target: { value: 'notes' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Set named input' }));
  await fireEvent.input(screen.getByLabelText('Input binding name'), { target: { value: 'optional-notes' } });
  await fireEvent.input(screen.getByLabelText('Producer task identity or batch alias'), { target: { value: 'research-b' } });
  await fireEvent.click(screen.getByLabelText('Accept absent input after settlement'));
  await fireEvent.click(screen.getByRole('button', { name: 'Set named input' }));
  await fireEvent.change(screen.getByLabelText('Input policy'), { target: { value: 'settled_outcome' } });
  await fireEvent.input(screen.getByLabelText('Input binding name'), { target: { value: 'failed' } });
  await fireEvent.input(screen.getByLabelText('Producer task identity or batch alias'), { target: { value: 'research-c' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Set named input' }));
  await fireEvent.input(screen.getByLabelText('New output name'), { target: { value: 'report' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Add output' }));
  await fireEvent.input(screen.getByLabelText('Project path'), { target: { value: '/workspace/composition' } });
  await fireEvent.click(screen.getByLabelText('Permit writes in this path'));
  await fireEvent.click(screen.getByRole('button', { name: 'Add path' }));
  await stage();
  const batch = await preview();
  expect(request).not.toHaveBeenCalled();
  expect(batch.expected_generation).toBe(7);
  expect(batch.changes).toMatchObject([{ kind: 'add', alias: 'compose', definition: {
    inputs: { notes: { kind: 'task_output', task: 'research-a', output: 'notes', optional: false }, 'optional-notes': { kind: 'task_output', task: 'research-b', output: 'notes', optional: true }, failed: { kind: 'settled_outcome', task: 'research-c' } },
    outputs: { report: { required: true, media_type: 'text/plain', max_bytes: 16384 } }, paths: [{ path: '/workspace/composition', write: true }],
  } }]);
  await fireEvent.click(screen.getByRole('button', { name: 'Submit reviewed batch' }));
  expect(request).toHaveBeenCalledExactlyOnceWith('/v1/workflows/work/changes', batch);
});

it('adopts a correction with only explicitly selected version-guarded future bindings', async () => {
  setup();
  await fireEvent.change(screen.getByLabelText('Change to stage'), { target: { value: 'adopt_correction' } });
  await fireEvent.change(screen.getByLabelText('Target task'), { target: { value: 'original' } });
  await fireEvent.change(screen.getByLabelText('Successful correction'), { target: { value: 'fixed' } });
  await fireEvent.change(screen.getByLabelText('Future consumer'), { target: { value: 'consumer' } });
  await fireEvent.input(screen.getByLabelText('Consumer input to rebind'), { target: { value: 'research' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Add binding redirect' }));
  await stage();
  expect((await preview()).changes).toEqual([{ kind: 'adopt_correction', task: 'original', correction: 'fixed', redirects: [{ consumer: 'consumer', input: 'research', expected_version: 3 }] }]);
});

it('creates corrective work after a failed original without requiring that original to succeed', async () => {
  const original: TaskReadDto = {
    ...workflow().tasks[0], frozen: null, result: null, result_restricted: false, origin: null,
    definition: {
      title: 'Failed research', description: '', instructions: 'Retry the independent research',
      capability: { id: 'model', operation: 'model.generate' }, inputs: {}, outputs: {},
      dependencies: ['prior-success'], required: true, corrective_of: null, paths: [], actions: null, tools: [],
    },
  };
  const { request } = setup(original);
  await stage();
  const batch = await preview();
  expect(batch.changes).toMatchObject([{ kind: 'add', definition: {
    corrective_of: 'original', dependencies: ['prior-success'],
  } }]);
  expect(original.definition.dependencies).toEqual(['prior-success']);
  await fireEvent.click(screen.getByRole('button', { name: 'Submit reviewed batch' }));
  expect(request).toHaveBeenCalledExactlyOnceWith('/v1/workflows/work/changes', batch);
});

it('retains refused drafts and requires a refreshed explicit review before a new request', async () => {
  const { request, rendered } = setup();
  request.mockRejectedValueOnce(new DaemonError('conflict', 'stale workflow generation', 409));
  await fireEvent.change(screen.getByLabelText('Change to stage'), { target: { value: 'wait' } });
  await fireEvent.input(screen.getByLabelText('Decision or missing input question'), { target: { value: 'Which correction should run?' } });
  await stage();
  const first = await preview();
  await fireEvent.click(screen.getByRole('button', { name: 'Submit reviewed batch' }));
  expect(screen.getByRole('alert').textContent).toContain('stale workflow generation');
  expect((screen.getByRole('button', { name: 'Preview complete batch' }) as HTMLButtonElement).disabled).toBe(true);
  expect(request).toHaveBeenCalledTimes(1);
  await rendered.rerender({ workflow: { ...workflow(), generation: 9 } });
  await fireEvent.click(screen.getByRole('button', { name: 'Refresh generation for review' }));
  const reviewed = await preview();
  expect(reviewed.changes).toEqual(first.changes);
  expect(reviewed.expected_generation).toBe(9);
  expect(reviewed.request_id).not.toBe(first.request_id);
  expect(request).toHaveBeenCalledTimes(1);
});

it('holds a lost-reply batch unchanged and replays its exact identity and guards', async () => {
  const { request } = setup();
  request.mockRejectedValueOnce(new Error('Reply lost')).mockResolvedValueOnce({});
  await fireEvent.change(screen.getByLabelText('Change to stage'), { target: { value: 'continue' } });
  await stage();
  const batch = await preview();
  await fireEvent.click(screen.getByRole('button', { name: 'Submit reviewed batch' }));
  expect(screen.queryByLabelText('Change to stage')).toBeNull();
  expect((screen.getByRole('button', { name: 'Close batch editor' }) as HTMLButtonElement).disabled).toBe(true);
  await fireEvent.click(screen.getByRole('button', { name: 'Replay exact batch' }));
  expect(request.mock.calls).toEqual([['/v1/workflows/work/changes', batch], ['/v1/workflows/work/changes', batch]]);
});

it('stages prospective limits and an explicit response decision with no implicit resume', async () => {
  setup();
  await fireEvent.change(screen.getByLabelText('Change to stage'), { target: { value: 'set_limits' } });
  await fireEvent.input(screen.getByLabelText('Total admitted tasks'), { target: { value: '300' } });
  await stage();
  await fireEvent.change(screen.getByLabelText('Change to stage'), { target: { value: 'dismiss_admission' } });
  await fireEvent.change(screen.getByLabelText('Target task'), { target: { value: 'original' } });
  await stage();
  const batch = await preview();
  expect(batch.changes).toEqual([{ kind: 'set_limits', limits: { ...workflow().limits, tasks: 300 } }, { kind: 'dismiss_admission', task: 'original' }]);
});

it('captures a future file input with the daemon limit and replays a lost upload exactly', async () => {
  const { request } = setup();
  request.mockResolvedValueOnce({ input_limit_bytes: 32 }).mockRejectedValueOnce(new Error('Upload reply lost'))
    .mockResolvedValueOnce({ input_id: 'captured-file', name: 'context.txt', bytes: 7, digest: 'exact-file-digest' });
  await fireEvent.change(screen.getByLabelText('Change to stage'), { target: { value: 'bind_input' } });
  await fireEvent.input(screen.getByLabelText('Supplied workflow input name'), { target: { value: 'file' } });
  const file = new File(['context'], 'context.txt', { type: 'text/plain' });
  Object.defineProperty(file, 'arrayBuffer', { value: async () => new TextEncoder().encode('context').buffer });
  await fireEvent.change(screen.getByLabelText('Capture replacement text file'), { target: { files: [file] } });
  expect((await screen.findByRole('alert')).textContent).toContain('Upload reply lost');
  await fireEvent.click(screen.getByRole('button', { name: 'Replay exact file capture' }));
  expect(request.mock.calls[2]).toEqual(request.mock.calls[1]);
  await stage();
  expect((await preview()).changes).toEqual([{ kind: 'bind_input', name: 'file', value: { kind: 'uploaded', input_id: 'captured-file' } }]);
  expect(request.mock.calls.every(([path]) => !path.endsWith('/changes'))).toBe(true);
});
