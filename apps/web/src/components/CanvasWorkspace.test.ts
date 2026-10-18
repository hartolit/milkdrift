import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import CanvasWorkspaceTestHarness from './CanvasWorkspaceTestHarness.svelte';
import { acceptedTask, session, workflow } from '../lib/canvas.test-fixtures';
import { addCanvasTask, cacheTask, createCanvas, updateInstructions, type CanvasState } from '../lib/canvas';
import type { ChangeWorkflowRequest, TaskReadDto } from '../lib/protocol.generated';
import { DaemonClient, DaemonError } from '../lib/transport';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });
function client(configured = session(), tasks: TaskReadDto[] = []) {
  const connection = new DaemonClient('http://127.0.0.1:3000', 'private-token');
  const request = vi.spyOn(connection, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/session') return configured as T;
    if (body) return { request_id: (body as { request_id: string }).request_id, workflow_id: 'workflow-a', generation: 6, aliases: {} } as T;
    return tasks.find((task) => path.endsWith(`/tasks/${task.id}`)) as T;
  });
  return { connection, request };
}
async function addTask(text: string) {
  await fireEvent.click(screen.getByRole('button', { name: '+ Add task' }));
  await fireEvent.input(screen.getByLabelText('What should this task do?'), { target: { value: text } });
}
function inspector() { return screen.getByRole('complementary', { name: 'Selected item' }); }
async function submit() { await fireEvent.submit(inspector().querySelector('form')!); }

it('keeps the empty workflow unsaved, then submits instructions with a visibly assigned model', async () => {
  const { connection, request } = client();
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection });
  expect(screen.getByRole('region', { name: 'Workflow canvas' })).toBeTruthy();
  expect(request).not.toHaveBeenCalled();
  await addTask('Research motion matching');
  expect((screen.getByLabelText('Model or agent') as HTMLSelectElement).selectedOptions[0].text).toBe('Research model');
  expect(inspector().querySelectorAll('textarea[required]')).toHaveLength(1);
  expect(screen.queryByLabelText('Input binding name')).toBeNull();
  await submit();
  await waitFor(() => expect(request.mock.calls.some(([path, body]) => path === '/v1/workflows' && !!body)).toBe(true));
  const sent = request.mock.calls.find(([, body]) => body)?.[1];
  expect(sent).toMatchObject({ title: 'Research motion matching', tasks: [{ definition: { instructions: 'Research motion matching', capability: { id: 'model-a', operation: 'model.generate' }, inputs: {}, tools: [], paths: [], actions: null } }] });
});

it('makes adding models discoverable without discarding the selected draft', async () => {
  const onmodels = vi.fn(), { connection } = client();
  let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, onmodels, onstate: (canvas) => state = canvas });
  await addTask('Keep these instructions');
  await fireEvent.click(screen.getByRole('button', { name: '+ Add model or agent' }));
  expect(onmodels).toHaveBeenCalledOnce();
  expect(state.tasks[0].definition.instructions).toBe('Keep these instructions');
});

it('connects text by keyboard selection without introducing protocol fields', async () => {
  const { connection } = client(); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, onstate: (canvas) => state = canvas });
  await addTask('Summarize this source');
  await fireEvent.click(screen.getByRole('button', { name: '+ Text' }));
  await fireEvent.input(screen.getByLabelText('Input text'), { target: { value: 'Captured research notes' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect to…' }));
  await fireEvent.change(screen.getByLabelText('Destination task'), { target: { value: state.tasks[0].id } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  expect(Object.values(state.tasks[0].definition.inputs)).toEqual([{ kind: 'inline', value: { kind: 'text', text: 'Captured research notes' } }]);
  expect(screen.getByRole('region', { name: 'Workflow canvas' })).toBeTruthy();
  for (const name of ['Input binding name', 'Input policy', 'Supplied input name', 'New output name', 'Batch alias']) expect(screen.queryByLabelText(name)).toBeNull();
});

it('connects a genuine draft output and its dependency in the same atomic drawing', async () => {
  const { connection } = client(); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, onstate: (canvas) => state = canvas });
  await addTask('Produce research'); const producer = state.tasks[0];
  await addTask('Compose research'); const consumer = state.tasks[1];
  await fireEvent.click(screen.getByRole('button', { name: 'Connect from Produce research' }));
  await fireEvent.change(screen.getByLabelText('Destination task'), { target: { value: consumer.id } });
  await fireEvent.change(screen.getByLabelText('Result to use'), { target: { value: 'final_text' } });
  await fireEvent.click(screen.getByRole('checkbox', { name: 'Continue if this optional result is absent' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  expect(state.tasks[0].definition.outputs).toEqual({ final_text: { required: false, media_type: 'text/plain', max_bytes: 1048576 } });
  expect(Object.values(state.tasks[1].definition.inputs)).toEqual([{ kind: 'task_output', task: producer.id, output: 'final_text', optional: true }]);
  expect(state.tasks[1].definition.dependencies).toEqual([producer.id]);
});

it('reuses an accepted queued file source after a fresh read', async () => {
  const original = acceptedTask(); original.definition.inputs.file = { kind: 'inline', value: { kind: 'uploaded', input_id: 'captured-file' } };
  original.inline_inputs.file = { availability: 'available', value: { restricted: false, kind: 'text_file', name: 'Cargo.toml', text: '[package]', text_truncated: false, digest: 'file-digest', artifact_id: null } };
  const { connection } = client(session(), [original]); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: workflow([original]), onstate: (canvas) => state = canvas });
  await screen.findByRole('button', { name: 'Connect from Cargo.toml' });
  await addTask('Inspect a captured manifest');
  await fireEvent.click(screen.getByRole('button', { name: 'Connect from Cargo.toml' }));
  await fireEvent.change(screen.getByLabelText('Destination task'), { target: { value: state.tasks[0].id } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  expect(Object.values(state.tasks[0].definition.inputs)).toEqual([{ kind: 'inline', value: { kind: 'uploaded', input_id: 'captured-file' } }]);
});

it('keeps the canvas camera while selection changes only the left inspector', async () => {
  const first = acceptedTask('a'), second = acceptedTask('b'); second.definition.title = 'Another task';
  const snapshot = workflow([first, second]), initial = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation });
  initial.camera = { x: 28, y: 37, zoom: .8 }; cacheTask(initial, first); cacheTask(initial, second);
  const { connection } = client(session(), [first, second]); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  await fireEvent.click(screen.getByRole('button', { name: 'Accepted task · queued' }));
  expect(within(inspector()).getByRole('heading', { name: 'Accepted task' })).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Another task · queued' }));
  expect(within(inspector()).getByRole('heading', { name: 'Another task' })).toBeTruthy();
  expect(screen.getByRole('region', { name: 'Workflow canvas' })).toBeTruthy();
  expect(state.camera).toEqual({ x: 28, y: 37, zoom: .8 });
});

it('labels frozen inputs from their captured value after a shared source is replaced', async () => {
  const original = acceptedTask(); original.state = 'completed'; original.started_at = 10; original.ended_at = 20;
  original.definition.inputs.context = { kind: 'supplied', name: 'shared', optional: false };
  const captured = { restricted: false, kind: 'text', name: null, text: 'Original shared context', text_truncated: false, digest: 'original', artifact_id: null };
  original.frozen = { inputs: { context: captured }, capability_generation: 3, actor_id: 'actor-a', grant_id: 'grant-a', grant_revision: 1, entered: true };
  const snapshot = workflow([original]); snapshot.supplied_inputs.shared = { availability: 'available', value: { ...captured, text: 'Replacement shared context', digest: 'replacement' } };
  const { connection } = client(session(), [original]);
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot });
  await fireEvent.click(await screen.findByRole('button', { name: 'Accepted task · completed' }));
  const frozen = within(inspector()).getByText('Inputs used at start').closest('details')!;
  expect(frozen.textContent).toContain('Original shared context');
  expect(frozen.textContent).not.toContain('Replacement shared context');
});

it('retains an exact pending drawing and recovers its receipt without resubmission', async () => {
  const configured = session(), { connection, request } = client(configured); let state!: CanvasState;
  const accepted = vi.fn(async () => {});
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/session') return configured as T;
    if (body) throw new Error('Reply lost');
    return { request_id: state.pending!.request.request_id, workflow_id: 'workflow-a', generation: 1, aliases: {} } as T;
  });
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, onstate: (canvas) => state = canvas, onaccepted: accepted });
  await addTask('Keep exact work'); await submit();
  await screen.findByRole('button', { name: 'Find submission receipt' });
  const frozen = JSON.stringify(state.pending!.request);
  await fireEvent.click(screen.getByRole('button', { name: 'Workflow' }));
  expect((screen.getByLabelText('Workflow name') as HTMLInputElement).disabled).toBe(true);
  expect((screen.getByLabelText('Finish when required work settles') as HTMLInputElement).disabled).toBe(true);
  await fireEvent.click(screen.getByRole('button', { name: 'Find submission receipt' }));
  await waitFor(() => expect(accepted).toHaveBeenCalledOnce());
  expect(request.mock.calls.filter(([, body]) => body)).toHaveLength(1);
  expect(JSON.stringify(request.mock.calls.find(([, body]) => body)?.[1])).toBe(frozen);
});

it('retains rejected drawings and requires explicit review after a stale generation', async () => {
  const snapshot = workflow(), configured = session(), { connection, request } = client(configured); let state!: CanvasState;
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/session') return configured as T;
    if (body) throw new DaemonError('conflict', 'Another client changed the workflow.', 409);
    return { ...snapshot, generation: 6 } as T;
  });
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, workflow: snapshot, onstate: (canvas) => state = canvas });
  await addTask('Retained after conflict'); await submit();
  await screen.findByRole('button', { name: 'Refresh workflow for review' });
  expect(state.tasks[0].definition.instructions).toBe('Retained after conflict');
  expect(state.pending).toBeNull();
  await fireEvent.click(screen.getByRole('button', { name: 'Refresh workflow for review' }));
  await waitFor(() => expect(state.conflict).toBe(false));
  expect(request.mock.calls.filter(([, body]) => body)).toHaveLength(1);
});

it('does not submit a drawing after fresh authentication changes actor', async () => {
  const configured = session(), { connection, request } = client({ ...configured, actor_id: 'other-actor' });
  const onerror = vi.fn();
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, onerror });
  await addTask('Do not transfer ownership'); await submit();
  await waitFor(() => expect(onerror).toHaveBeenCalled());
  expect(request.mock.calls.filter(([, body]) => body)).toHaveLength(0);
  expect(onerror.mock.calls[0][0]).toMatchObject({ status: 401 });
});

it('submits repaired tasks with dismissal while preserving a separate human wait', async () => {
  const original = acceptedTask(); original.state = 'completed'; original.started_at = 1; original.ended_at = 2;
  const snapshot = workflow([original]); snapshot.wait_question = 'Await budget approval'; snapshot.admissions = [{ task_id: original.id, status: 'rejected', response_digest: 'proposal', reason: 'Invalid proposal', receipt_key: null, aliases: {} }];
  const { connection, request } = client(session(), [original]); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, onstate: (canvas) => state = canvas });
  await waitFor(() => expect(state.details).toHaveLength(1));
  await fireEvent.click(screen.getByRole('button', { name: 'Accepted task · completed' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Dismiss reviewed proposal' }));
  await addTask('Reviewed replacement task'); await submit();
  await waitFor(() => expect(request.mock.calls.some(([, body]) => body)).toBe(true));
  const body = request.mock.calls.find(([, body]) => body)![1] as ChangeWorkflowRequest;
  expect(body.changes.map((change) => change.kind)).toEqual(['add', 'dismiss_admission']);
  expect(body.changes.some((change) => change.kind === 'continue')).toBe(false);
});

it('explains a start racing a prospective edit and keeps the rejected instructions', async () => {
  const original = acceptedTask(), snapshot = workflow([original]), configured = session(), initial = createCanvas(configured, { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: 5 }); cacheTask(initial, original);
  const { connection, request } = client(configured, [original]); let state!: CanvasState;
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/session') return configured as T;
    if (body) throw new DaemonError('conflict', 'Task already started', 409);
    if (path.endsWith(`/tasks/${original.id}`)) return { ...original, state: 'running', started_at: 99 } as T;
    return { ...snapshot, generation: 6 } as T;
  });
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  await fireEvent.click(screen.getByRole('button', { name: 'Accepted task · queued' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Edit task' }));
  await fireEvent.input(screen.getByLabelText('What should this task do?'), { target: { value: 'Prospective edit' } });
  await submit(); await screen.findByRole('button', { name: 'Refresh workflow for review' });
  await fireEvent.click(screen.getByRole('button', { name: 'Refresh workflow for review' }));
  await waitFor(() => expect(state.error).toContain('has started'));
  expect(state.tasks[0].definition.instructions).toBe('Prospective edit');
  expect(state.conflict).toBe(true);
});

it('retains camera and draft when switching to the accessible list and back', async () => {
  const configured = session(), initial = createCanvas(configured, { kind: 'new' }), task = addCanvasTask(initial, configured); updateInstructions(initial, task, 'Keyboard task');
  const { connection } = client(configured); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, initial, onstate: (canvas) => state = canvas });
  const camera = { ...state.camera };
  await fireEvent.click(screen.getByRole('button', { name: 'List' }));
  expect(screen.getByRole('region', { name: 'Tasks and inputs' })).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Canvas' }));
  expect(state.camera).toEqual(camera);
  expect(state.tasks[0].definition.instructions).toBe('Keyboard task');
});

it('shares the finite output allowance visibly before connecting a second genuine result', async () => {
  const configured = session(); configured.authoring[0].outputs = configured.authoring[0].outputs.map((port) => ({ ...port, max_bytes: 2097152 }));
  const { connection } = client(configured); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, onstate: (canvas) => state = canvas });
  await addTask('Producer'); const producer = state.tasks[0];
  await addTask('First consumer'); const first = state.tasks[1];
  await fireEvent.click(screen.getByRole('button', { name: 'Connect from Producer' }));
  await fireEvent.change(screen.getByLabelText('Destination task'), { target: { value: first.id } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  expect(state.tasks[0].definition.outputs.model_response.max_bytes).toBe(2097152);
  await addTask('Second consumer'); const second = state.tasks[2];
  await fireEvent.click(screen.getByRole('button', { name: 'Connect from Producer' }));
  await fireEvent.change(screen.getByLabelText('Destination task'), { target: { value: second.id } });
  await fireEvent.change(screen.getByLabelText('Result to use'), { target: { value: 'final_text' } });
  expect(screen.getByText(/shares its 2 MiB result allowance/)).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  expect(Object.values(state.tasks.find((task) => task.id === producer.id)!.definition.outputs).map((port) => port.max_bytes)).toEqual([1048576, 1048576]);
});

it('selects a correction and named future consumers without typing task or input identities', async () => {
  const failed = acceptedTask('failed'); failed.state = 'failed'; failed.started_at = 1; failed.ended_at = 2; failed.definition.title = 'Failed research';
  const replacement = acceptedTask('replacement'); replacement.state = 'completed'; replacement.started_at = 3; replacement.ended_at = 4; replacement.definition.title = 'Repaired research'; replacement.definition.corrective_of = failed.id;
  const consumer = acceptedTask('consumer'); consumer.definition.title = 'Waiting composition'; consumer.definition.inputs.source = { kind: 'task_output', task: failed.id, output: 'model_response', optional: false }; consumer.definition.dependencies = [failed.id];
  const tasks = [failed, replacement, consumer], snapshot = workflow(tasks), initial = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation }); for (const task of tasks) cacheTask(initial, task);
  const { connection, request } = client(session(), tasks);
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, initial });
  await fireEvent.click(screen.getByRole('button', { name: 'Failed research · failed' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Use correction for future work' }));
  await fireEvent.change(screen.getByLabelText('Successful correction'), { target: { value: replacement.id } });
  await fireEvent.click(screen.getByRole('checkbox', { name: 'Waiting composition' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Use selected correction' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Apply changes' }));
  await waitFor(() => expect(request.mock.calls.some(([, body]) => body)).toBe(true));
  expect((request.mock.calls.find(([, body]) => body)![1] as ChangeWorkflowRequest).changes).toEqual([{ kind: 'adopt_correction', task: failed.id, correction: replacement.id, redirects: [{ consumer: consumer.id, expected_version: consumer.version, input: 'source' }] }]);
});

it('clears cached observations without discarding draft content', async () => {
  const task = acceptedTask(), configured = session(), snapshot = workflow([task]), initial = createCanvas(configured, { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation }); cacheTask(initial, task);
  const added = addCanvasTask(initial, configured); updateInstructions(initial, added, 'Keep my draft');
  const { connection } = client(configured, [task]); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  await fireEvent.click(screen.getByRole('button', { name: 'Workflow' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Clear cached details' }));
  expect(state.details).toHaveLength(0);
  expect(state.tasks[0].definition.instructions).toBe('Keep my draft');
  await fireEvent.click(screen.getByRole('button', { name: 'Accepted task · queued' }));
  await waitFor(() => expect(state.details).toHaveLength(1));
});


it('releases discarded draft layout and binding entries for repeated authoring', async () => {
  const { connection } = client(); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, onstate: (canvas) => state = canvas });
  for (let round = 0; round < 4; round++) {
    await addTask('Disposable draft');
    await fireEvent.click(screen.getByRole('button', { name: '+ Add input' }));
    const source = state.sources[0], task = state.tasks[0];
    expect(state.bindings).toEqual({ [`${task.id}:${source.id}`]: source.id });
    await fireEvent.click(screen.getByRole('button', { name: 'Disposable draft · Unsaved changes' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Discard task' }));
    expect(state.bindings).toEqual({});
    expect(state.positions[task.id]).toBeUndefined();
    await fireEvent.click(screen.getByRole('button', { name: 'Text input · Unsaved changes' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Discard input' }));
    expect(state.positions).toEqual({});
  }
});

it('preserves the whole root when granting an explicitly available root project', async () => {
  const configured = session(); configured.project_roots = [{ path: '/', label: 'Root project', read: true, write: false }];
  const { connection } = client(configured); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, onstate: (canvas) => state = canvas });
  await addTask('Inspect selected project');
  await fireEvent.change(screen.getByLabelText('Project'), { target: { value: '/' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Allow project access' }));
  expect(state.tasks[0].definition.paths).toEqual([{ path: '/', write: false }]);
  expect(state.tasks[0].definition.tools).toEqual([{ kind: 'read_file' }]);
});


it('a correction retains the original source connections when the new draft becomes reactive', async () => {
  const original = acceptedTask(); original.state = 'stopped'; original.definition.inputs.context = { kind: 'supplied', name: 'original-context', optional: false };
  const snapshot = workflow([original]), configured = session(), initial = createCanvas(configured, { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation }); cacheTask(initial, original);
  const { connection } = client(configured, [original]); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  await fireEvent.click(screen.getByRole('button', { name: 'Accepted task · stopped' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Create correction' }));
  await fireEvent.input(screen.getByLabelText('What should this task do?'), { target: { value: 'Repair this analysis' } });
  expect(state.tasks[0].definition.inputs).toEqual(original.definition.inputs);
  expect(state.tasks[0].definition.corrective_of).toBe(original.id);
});


it('refuses reusing a restricted source and exposes no ungranted project picker', async () => {
  const original = acceptedTask(); original.definition.inputs.context = { kind: 'supplied', name: 'private-context', optional: false };
  original.supplied_inputs = { 'private-context': { availability: 'restricted', value: null } };
  const snapshot = workflow([original]); snapshot.supplied_inputs = original.supplied_inputs;
  const configured = session(), initial = createCanvas(configured, { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation }); cacheTask(initial, original);
  const { connection, request } = client(configured, [original]); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  await addTask('Consume available context');
  expect(screen.queryByLabelText('Project')).toBeNull();
  expect(screen.getByText('No project folders are available with this connection’s permissions.')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Connect from Required input' }));
  await fireEvent.change(screen.getByLabelText('Destination task'), { target: { value: state.tasks[0].id } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  expect(screen.getByRole('alert').textContent).toContain('restricted or expired');
  expect(state.tasks[0].definition.inputs).toEqual({});
  expect(request.mock.calls.some(([, body]) => !!body)).toBe(false);
});
