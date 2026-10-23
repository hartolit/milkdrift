import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import CanvasWorkspaceTestHarness from './CanvasWorkspaceTestHarness.svelte';
import { acceptedTask, session, workflow, project, projectCanvas as createCanvas } from '../lib/canvas.test-fixtures';
import { addCanvasTask, cacheTask, copy, freezeCanvas, updateInstructions, type CanvasState } from '../lib/canvas';
import type { ChangeWorkflowRequest, TaskProjectBindingDto, TaskReadDto } from '../lib/protocol.generated';
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

it('pauses immediately without submitting an unfinished drawing or moving its selection', async () => {
  const existing = acceptedTask(), snapshot = workflow([existing]);
  const initial = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation });
  addCanvasTask(initial, session());
  const selected = initial.selected;
  initial.camera = { x: 28, y: 37, zoom: .8 };
  const { connection, request } = client(session(), [existing]), accepted = vi.fn();
  let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas, onaccepted: accepted });
  await fireEvent.click(screen.getByRole('button', { name: 'Pause workflow' }));
  await waitFor(() => expect(state.pending).toBeNull());
  expect(request.mock.calls.filter(([, body]) => body).map(([, body]) => body)).toEqual([
    expect.objectContaining({ expected_generation: 5, changes: [{ kind: 'wait', question: 'Paused by you.' }] }),
  ]);
  expect(accepted).not.toHaveBeenCalled();
  expect(state.tasks).toHaveLength(1);
  expect(state.tasks[0].definition.instructions).toBe('');
  expect(state.selected).toBe(selected);
  expect(state.camera).toEqual({ x: 28, y: 37, zoom: .8 });
  expect(state.target).toMatchObject({ generation: 6 });
});

it('keeps a deferred frozen task immutable and requests stop without inventing settlement', async () => {
  const deferred = acceptedTask();
  deferred.reason = 'Project is in use'; deferred.started_at = 10;
  deferred.frozen = { inputs: {}, capability_generation: 3, actor_id: 'actor-a', grant_id: 'grant-a', grant_revision: 1, entered: false };
  const snapshot = workflow([deferred]), initial = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation });
  const draft = addCanvasTask(initial, session()); updateInstructions(initial, draft, 'Keep my unrelated idea');
  cacheTask(initial, deferred); initial.selected = deferred.id;
  const { connection, request } = client(session(), [deferred]), accepted = vi.fn();
  let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas, onaccepted: accepted });
  expect(within(inspector()).getByText('Waiting before entry')).toBeTruthy();
  expect(within(inspector()).getByText('Project is in use', { selector: '.notice' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Edit task' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Remove unstarted task' })).toBeNull();
  expect(within(inspector()).getByText('No entry recorded')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Request stop' }));
  await waitFor(() => expect(state.pending).toBeNull());
  expect(request.mock.calls.find(([, body]) => body)?.[1]).toEqual({ request_id: deferred.stop_request_id, expected_generation: 5, changes: [{ kind: 'stop', task: deferred.id }] });
  expect(state.tasks[0].definition.instructions).toBe('Keep my unrelated idea');
  expect(state.selected).toBe(deferred.id); expect(accepted).not.toHaveBeenCalled();
  expect(screen.getByText('Stop requested. Waiting for the daemon to report the outcome.')).toBeTruthy();
  expect(within(inspector()).getByText('No settlement recorded')).toBeTruthy();
});

it('recovers the exact resume request while retaining the selected draft and visible pause', async () => {
  const snapshot = workflow(); snapshot.wait_question = 'Choose the publication date.';
  const initial = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation });
  const draft = addCanvasTask(initial, session()); updateInstructions(initial, draft, 'Keep this unsubmitted task');
  const { connection, request } = client(), accepted = vi.fn(); let state!: CanvasState;
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (body) throw new Error('Resume reply lost');
    if (path.startsWith('/v1/workflow-requests/')) return { request_id: state.pending!.request.request_id, workflow_id: snapshot.id, generation: 6, aliases: {} } as T;
    throw new Error(`Unexpected read: ${path}`);
  });
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas, onaccepted: accepted });
  expect(screen.getByRole('region', { name: 'Workflow pause' }).textContent).toContain(snapshot.wait_question);
  await fireEvent.click(screen.getByRole('button', { name: 'Resume workflow' }));
  await screen.findByRole('button', { name: 'Find submission receipt' });
  const pending = copy(state.pending);
  expect(pending).toMatchObject({ control: true, path: '/v1/workflows/workflow-a/changes', request: { expected_generation: 5, changes: [{ kind: 'continue' }] } });
  expect((screen.getByRole('button', { name: 'Resume workflow' }) as HTMLButtonElement).disabled).toBe(true);
  await fireEvent.click(screen.getByRole('button', { name: 'Find submission receipt' }));
  await waitFor(() => expect(state.pending).toBeNull());
  expect(request.mock.calls.filter(([, body]) => body)).toHaveLength(1);
  expect(request).toHaveBeenLastCalledWith(`/v1/workflow-requests/${pending!.request.request_id}`, undefined);
  expect(state.tasks[0].definition.instructions).toBe('Keep this unsubmitted task');
  expect(state.selected).toBe(draft.id); expect(accepted).not.toHaveBeenCalled();
});

it('uses the current workflow for a control without rebasing an older drawing', async () => {
  const snapshot = workflow(); snapshot.generation = 8;
  const initial = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: 5 });
  const draft = addCanvasTask(initial, session()); updateInstructions(initial, draft, 'Retain my earlier work');
  initial.changes = [{ kind: 'bind_input', name: 'source', value: { kind: 'text', text: 'Keep this staged source' } }];
  const staged = copy(initial.changes);
  const { connection, request } = client(); let state!: CanvasState;
  request.mockImplementation(async <T>(_path: string, body?: unknown): Promise<T> => ({ request_id: (body as { request_id: string }).request_id, workflow_id: snapshot.id, generation: 9, aliases: {} }) as T);
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  await fireEvent.click(screen.getByRole('button', { name: 'Pause workflow' }));
  await waitFor(() => expect(state.pending).toBeNull());
  expect(request).toHaveBeenCalledExactlyOnceWith('/v1/workflows/workflow-a/changes', expect.objectContaining({ expected_generation: 8, changes: [{ kind: 'wait', question: 'Paused by you.' }] }));
  expect(state.target).toMatchObject({ generation: 5 });
  expect(state.tasks[0].definition.instructions).toBe('Retain my earlier work');
  expect(state.changes).toEqual(staged); expect(state.selected).toBe(draft.id);
});

it('leaves an uncertain authoring request intact when a workflow control is attempted', async () => {
  const snapshot = workflow(), initial = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation });
  const draft = addCanvasTask(initial, session()); updateInstructions(initial, draft, 'An already submitted task');
  initial.pending = freezeCanvas(initial, session());
  const pending = copy(initial.pending), { connection, request } = client(); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  const pause = screen.getByRole('button', { name: 'Pause workflow' }) as HTMLButtonElement;
  expect(pause.disabled).toBe(true); await fireEvent.click(pause);
  expect(request).not.toHaveBeenCalled(); expect(state.pending).toEqual(pending);
  expect(state.tasks[0].definition.instructions).toBe('An already submitted task');
});

it('refuses a stale control without automatic retry or discarding its unrelated drawing', async () => {
  const snapshot = workflow(); snapshot.wait_question = 'A decision that may have changed.';
  const initial = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation });
  const draft = addCanvasTask(initial, session()); updateInstructions(initial, draft, 'Do not submit this idea');
  const { connection, request } = client(); let state!: CanvasState;
  request.mockRejectedValue(new DaemonError('conflict', 'A newer decision requires review.', 409));
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  await fireEvent.click(screen.getByRole('button', { name: 'Resume workflow' }));
  await waitFor(() => expect(state.pending).toBeNull());
  expect(request).toHaveBeenCalledOnce();
  expect(screen.getByText('The workflow changed. Review its current state before choosing this action again. Your drawing is retained.')).toBeTruthy();
  expect(state.target).toMatchObject({ generation: 5 }); expect(state.selected).toBe(draft.id);
  expect(state.tasks[0].definition.instructions).toBe('Do not submit this idea');
  expect(screen.queryByRole('button', { name: 'Replay exact submission' })).toBeNull();
});

it('keeps supplying input unavailable until an uncertain control receipt is recovered', async () => {
  const task = acceptedTask(); task.definition.inputs.document = { kind: 'supplied', name: 'source', optional: false };
  const snapshot = workflow([task]); snapshot.wait_question = 'Choose when to continue.';
  const initial = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation });
  cacheTask(initial, task); initial.selected = 'supplied:source'; initial.inspector = true;
  initial.pending = { path: '/v1/workflows/workflow-a/changes', control: true, request: { request_id: 'pending-control', expected_generation: 5, changes: [{ kind: 'continue' }] } };
  const { connection, request } = client(); let state!: CanvasState;
  request.mockImplementation(async <T>(path: string): Promise<T> => {
    if (path === '/v1/workflow-requests/pending-control') return { request_id: 'pending-control', workflow_id: snapshot.id, generation: 6, aliases: {} } as T;
    throw new Error(`Unexpected request: ${path}`);
  });
  render(CanvasWorkspaceTestHarness, { session: session(), client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  const field = screen.getByLabelText('Replacement text') as HTMLTextAreaElement;
  const supply = screen.getByRole('button', { name: 'Supply input' }) as HTMLButtonElement;
  expect(field.disabled).toBe(true); expect(supply.disabled).toBe(true);
  await fireEvent.click(supply); expect(state.changes).toEqual([]);
  await fireEvent.click(screen.getByRole('button', { name: 'Find submission receipt' }));
  await waitFor(() => expect(supply.disabled).toBe(false)); expect(field.disabled).toBe(false);
  expect(state.changes).toEqual([]); expect(state.selected).toBe('supplied:source');
  await fireEvent.input(field, { target: { value: 'The source to use later' } }); await fireEvent.click(supply);
  expect(state.changes).toEqual([{ kind: 'bind_input', name: 'source', value: { kind: 'text', text: 'The source to use later' } }]);
  expect(request).toHaveBeenCalledExactlyOnceWith('/v1/workflow-requests/pending-control', undefined);
});

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

it('expresses read-only project access as an explicit complete selection without a root editor', async () => {
  const configured = session();
  const { connection } = client(configured); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, onstate: (canvas) => state = canvas });
  await addTask('Inspect selected project');
  await fireEvent.change(screen.getByLabelText('Task project access'), { target: { value: 'read' } });
  expect(state.tasks[0].definition.execution).toEqual({ kind: 'explicit', expected_generation: 1, settings: { environment: { kind: 'host' }, allow_changes: false }, access: 'read', tools: [{ kind: 'read_file' }] });
  expect(state.tasks[0].definition.paths).toEqual([]);
  expect(state.tasks[0].definition.tools).toEqual([]);
  expect(screen.queryByLabelText('Project')).toBeNull();
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

it.each(['none', 'read', 'write'] as const)('a correction preserves %s access and its environment after project defaults change', async (access) => {
  const original = acceptedTask(); original.state = 'stopped';
  const binding: TaskProjectBindingDto = {
    project_id: project.id, generation: 1, folder: { requested: project.folder, physical: project.folder, identity: 'original-folder' },
    settings: { environment: { kind: 'host' }, allow_changes: access === 'write' }, access,
    paths: access === 'none' ? [] : [{ path: project.folder, write: access === 'write' }], scopes: [],
    tools: access === 'none' ? [] : [{ kind: 'read_file' }, ...(access === 'write' ? [{ kind: 'write_file' } as const] : [])],
  };
  original.definition.execution = { kind: 'bound', binding };
  const historical = JSON.stringify(original.definition);
  const currentProject = { ...project, generation: 4, settings: { environment: { kind: 'host' as const }, allow_changes: true } };
  const snapshot = workflow([original]), configured = session(), initial = createCanvas(configured, { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation }); cacheTask(initial, original);
  const { connection, request } = client(configured, [original]); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, workflow: snapshot, currentProject, initial, onstate: (canvas) => state = canvas });
  await fireEvent.click(screen.getByRole('button', { name: 'Accepted task · stopped' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Create correction' }));
  await fireEvent.input(screen.getByLabelText('What should this task do?'), { target: { value: 'Repair with the original permissions' } });
  const expected = { kind: 'explicit', expected_generation: 4, settings: binding.settings, access, tools: binding.tools };
  expect(state.tasks[0].definition.execution).toEqual(expected);
  expect(state.tasks[0].definition.paths).toEqual(binding.paths);
  expect((screen.getByLabelText('Task project access') as HTMLSelectElement).selectedOptions).toHaveLength(1);
  await submit();
  await waitFor(() => expect(request.mock.calls.some(([, body]) => !!body)).toBe(true));
  expect((request.mock.calls.find(([, body]) => body)![1] as ChangeWorkflowRequest).changes).toMatchObject([{ kind: 'add', definition: { execution: expected, paths: binding.paths } }]);
  expect(JSON.stringify(original.definition)).toBe(historical);
});

it('refuses a correction whose original execution selection is hidden', async () => {
  const original = acceptedTask(); original.state = 'stopped'; original.definition.execution = { kind: 'restricted' };
  const snapshot = workflow([original]), configured = session(), initial = createCanvas(configured, { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation }); cacheTask(initial, original);
  const { connection, request } = client(configured, [original]); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: configured, client: connection, workflow: snapshot, initial, onstate: (canvas) => state = canvas });
  await fireEvent.click(screen.getByRole('button', { name: 'Accepted task · stopped' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Create correction' }));
  expect(state.tasks).toHaveLength(0);
  expect(state.error).toContain('execution selection is restricted');
  expect(request.mock.calls.filter(([, body]) => body)).toHaveLength(0);
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
  expect(screen.getByLabelText('Task project access')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Connect from Required input' }));
  await fireEvent.change(screen.getByLabelText('Destination task'), { target: { value: state.tasks[0].id } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  expect(screen.getByRole('alert').textContent).toContain('restricted or expired');
  expect(state.tasks[0].definition.inputs).toEqual({});
  expect(request.mock.calls.some(([, body]) => !!body)).toBe(false);
});
