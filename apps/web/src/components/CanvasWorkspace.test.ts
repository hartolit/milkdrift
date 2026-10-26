import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import CanvasWorkspaceTestHarness from './CanvasWorkspaceTestHarness.svelte';
import { acceptedTask, session, workflow, projectCanvas as createCanvas, topology } from '../lib/canvas.test-fixtures';
import { addCanvasTask, addConnection, addSource, canvasDocument, copy, updateInstructions, useUsualModel, capabilityKey, type CanvasState } from '../lib/canvas';
import type { AuthoringDocumentDto, AuthoringReviewDto, CheckpointAuthoringRequest, ReviewAuthoringRequest, RunAuthoringRequest, TaskReadDto, WorkflowReadDto } from '../lib/protocol.generated';
import { DaemonClient, DaemonError } from '../lib/transport';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });
function drawing(snapshot = workflow()) { return createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation }); }
async function mount(initial = drawing(), tasks: TaskReadDto[] = [], snapshot: WorkflowReadDto = workflow(tasks)) {
  const client = new DaemonClient('http://127.0.0.1:3000', 'private-token');
  let document = canvasDocument(initial), generation = 0;
  const receipt = (request_id: string) => ({ request_id, workflow_id: snapshot.id, generation: 6, aliases: {} });
  const request = vi.spyOn(client, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path.endsWith('/authoring/review')) { const review = body as ReviewAuthoringRequest; return { ...review, expected_generation: 5, issues: [] } as T; }
    if (path.endsWith('/authoring/run')) { const run = body as RunAuthoringRequest; document = { ...document, tasks: document.tasks.filter((task) => !run.tasks.includes(task.id)), accepted: { ...document.accepted, ...Object.fromEntries(run.tasks.map((id) => [id, `accepted_${id}`])) } }; generation++; return { ...receipt(run.request_id), aliases: document.accepted } as T; }
    if (path.endsWith('/authoring')) return { generation, digest: String(generation), document: copy(document) } as T;
    if (path.endsWith('/positions')) return { generation: 0, digest: 'positions', positions: copy(initial.positions) } as T;
    if (path.endsWith('/topology')) return topology(tasks) as T;
    if (path.includes('/tasks/')) return tasks.find((task) => path.endsWith(`/tasks/${task.id}`)) as T;
    if (path.startsWith('/v1/workflow-requests/')) return receipt(path.split('/').at(-1)!) as T;
    if (body) return receipt((body as { request_id: string }).request_id) as T;
    throw new Error(`Unexpected read ${path}`);
  });
  const put = vi.spyOn(client, 'put').mockImplementation(async <T>(path: string, body: unknown): Promise<T> => {
    if (path.endsWith('/authoring')) { document = copy((body as CheckpointAuthoringRequest).document); generation++; return { generation, digest: String(generation), document: copy(document) } as T; }
    return { generation: 1, digest: 'positions-1', positions: (body as { positions: unknown }).positions } as T;
  });
  const accepted = vi.fn(async () => {}); let state!: CanvasState;
  render(CanvasWorkspaceTestHarness, { session: session(), client, workflow: snapshot, initial, onstate: (value) => state = value, onaccepted: accepted });
  await screen.findByText('Saved', { selector: '.save-state' });
  return { client, request, put, accepted, state, saved: () => document };
}
async function add(text = '') {
  await fireEvent.click(screen.getByRole('button', { name: '+ Add task' }));
  const cards = screen.getAllByRole('button', { name: 'New task · Draft' });
  await fireEvent.click(cards.at(-1)!);
  if (text) await fireEvent.input(screen.getByLabelText('What should this task do?'), { target: { value: text } });
}
function inspector() { return screen.getByRole('complementary', { name: 'Selected item' }); }

it('places a blank card without opening details or stealing the focused instructions', async () => {
  const { state, request, put } = await mount();
  await fireEvent.click(screen.getByRole('button', { name: '+ Add task' }));
  expect(screen.queryByRole('complementary', { name: 'Selected item' })).toBeNull(); expect(state.tasks[0].definition.capability.id).toBe('');
  await fireEvent.click(screen.getByRole('button', { name: 'New task · Draft' }));
  const input = screen.getByLabelText('What should this task do?'); input.focus();
  await fireEvent.input(input, { target: { value: 'Keep this thought' } }); const selected = state.selected;
  await fireEvent.click(screen.getByRole('button', { name: '+ Add task' }));
  expect(document.activeElement).toBe(input); expect(state.selected).toBe(selected); expect(state.tasks).toHaveLength(2);
  await waitFor(() => expect(put.mock.calls.some(([path]) => path.endsWith('/authoring'))).toBe(true));
  expect(request.mock.calls.filter(([, body]) => body)).toEqual([]);
});
it('highlights only unassigned cards when one usual model is applied to existing and future tasks', async () => {
  const initial = drawing(); addCanvasTask(initial, session()); const selected = addCanvasTask(initial, session(), session().capabilities[0]);
  const { state } = await mount(initial);
  await fireEvent.click(screen.getByRole('button', { name: 'Usual model…' }));
  await fireEvent.change(screen.getByLabelText('Model for this canvas'), { target: { value: capabilityKey(session().capabilities[0]) } });
  await fireEvent.click(screen.getByRole('button', { name: 'Use for these 1 tasks and new tasks' }));
  expect(state.highlighted).toEqual([state.tasks[0].id]); expect(state.highlighted).not.toContain(selected.id);
  await fireEvent.click(screen.getByRole('button', { name: '+ Add task' })); expect(state.tasks[2].definition.capability).toEqual(session().capabilities[0]);
});
it('connects physical generic ports by keyboard without a source or destination form', async () => {
  const initial = drawing(); addCanvasTask(initial, session()); addCanvasTask(initial, session()); const { state } = await mount(initial);
  await fireEvent.click(screen.getAllByRole('button', { name: 'Connect Output from New task' })[0]);
  await fireEvent.click(screen.getAllByRole('button', { name: 'Input on New task' })[1]);
  await waitFor(() => expect(state.connections).toHaveLength(1)); expect(state.connections[0]).toMatchObject({ input: null, output: null });
  expect(screen.queryByLabelText('Destination task')).toBeNull(); expect(screen.queryByRole('complementary', { name: 'Selected item' })).toBeNull();
  expect(document.querySelector('.unfinished-wire')).not.toBeNull();
});
it('keeps canvas and selected draft while a direct pause accepts independently of saving', async () => {
  const initial = drawing(), draft = addCanvasTask(initial, session()); initial.selected = draft.id; initial.inspector = true; initial.camera = { x: 28, y: 37, zoom: .8 };
  const { state, request, accepted } = await mount(initial);
  await fireEvent.click(screen.getByRole('button', { name: 'Pause workflow' })); await waitFor(() => expect(state.pending).toBeNull());
  expect(request.mock.calls.filter(([path, body]) => path.endsWith('/changes') && body).map(([, body]) => body)).toEqual([expect.objectContaining({ changes: [{ kind: 'wait', question: 'Paused by you.' }] })]);
  expect(state.tasks).toHaveLength(1); expect(state.selected).toBe(draft.id); expect(state.camera).toEqual({ x: 28, y: 37, zoom: .8 }); expect(accepted).not.toHaveBeenCalled();
});
it('keeps a frozen deferred task immutable and stop acknowledgement separate from settlement', async () => {
  const task = acceptedTask(); task.reason = 'Project is in use'; task.started_at = 10; task.frozen = { inputs: {}, capability_generation: 3, actor_id: 'actor-a', grant_id: 'grant-a', grant_revision: 1, entered: false };
  const { state, request } = await mount(drawing(), [task]);
  await fireEvent.click(screen.getByRole('button', { name: 'Accepted task · waiting before entry' })); await screen.findByText('Waiting before entry', { selector: '.badge' });
  expect(screen.queryByRole('button', { name: 'Edit task' })).toBeNull(); expect(within(inspector()).getByText('No entry recorded')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Request stop' })); await waitFor(() => expect(state.pending).toBeNull());
  expect(request.mock.calls.find(([path]) => path.endsWith('/changes'))?.[1]).toEqual({ request_id: task.stop_request_id, expected_generation: 5, changes: [{ kind: 'stop', task: task.id }] });
  expect(screen.getByText('Stop requested. Waiting for the daemon to report the outcome.')).toBeTruthy(); expect(within(inspector()).getByText('No settlement recorded')).toBeTruthy();
});
it('retains exact uncertain control and recovers its receipt without replacing saved authoring', async () => {
  const snapshot = workflow(); snapshot.wait_question = 'Choose publication date'; const initial = drawing(snapshot); addCanvasTask(initial, session());
  const { request, state, accepted } = await mount(initial, [], snapshot), original = request.getMockImplementation()!;
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => { if (path.endsWith('/changes')) throw new Error('reply lost'); return original(path, body) as Promise<T>; });
  await fireEvent.click(screen.getByRole('button', { name: 'Resume workflow' })); await screen.findByRole('button', { name: 'Find submission receipt' }); const pending = copy(state.pending);
  await fireEvent.click(screen.getByRole('button', { name: 'Find submission receipt' })); await waitFor(() => expect(state.pending).toBeNull());
  expect(request.mock.calls.filter(([path]) => path.endsWith('/changes'))).toHaveLength(1); expect(request.mock.calls.some(([path]) => path.endsWith(pending!.request.request_id))).toBe(true); expect(state.tasks).toHaveLength(1); expect(accepted).not.toHaveBeenCalled();
});
it('runs a chosen connected subset while a blank idea remains saved', async () => {
  const initial = drawing(); useUsualModel(initial, session(), capabilityKey(session().capabilities[0]), false);
  const work = addCanvasTask(initial, session()), idea = addCanvasTask(initial, session()); updateInstructions(initial, work, 'Research a useful question');
  const { state, request, saved } = await mount(initial);
  await fireEvent.click(screen.getByRole('button', { name: 'Choose tasks to run' })); const scope = screen.getByRole('region', { name: 'Tasks to run' });
  await fireEvent.click(within(scope).getByLabelText('New task')); await fireEvent.click(within(scope).getByRole('button', { name: 'Review this plan' }));
  await screen.findByRole('button', { name: 'Run reviewed plan' }); await fireEvent.click(screen.getByRole('button', { name: 'Run reviewed plan' }));
  await waitFor(() => expect(state.pending).toBeNull()); expect(request.mock.calls.find(([path]) => path.endsWith('/authoring/run'))?.[1]).toMatchObject({ tasks: [work.id] });
  expect(saved().tasks.map((task) => task.id)).toEqual([idea.id]); expect(state.tasks.map((task) => task.id)).toEqual([idea.id]);
});
it('shows daemon-added predecessors before submitting and requires an explicit candidate choice', async () => {
  const initial = drawing(), predecessor = addCanvasTask(initial, session()), consumer = addCanvasTask(initial, session()); updateInstructions(initial, predecessor, 'Research'); updateInstructions(initial, consumer, 'Compare');
  addConnection(initial, session(), predecessor.id, consumer.id);
  const { request } = await mount(initial), original = request.getMockImplementation()!;
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => path.endsWith('/authoring/review') ? ({ ...(body as ReviewAuthoringRequest), tasks: [consumer.id, predecessor.id], expected_generation: 5, issues: [] } satisfies AuthoringReviewDto) as T : original(path, body) as Promise<T>);
  await fireEvent.click(screen.getByRole('button', { name: 'Choose tasks to run' })); await fireEvent.click(screen.getByLabelText('Research'));
  await fireEvent.click(screen.getByRole('button', { name: 'Run pending plan' })); await screen.findByRole('button', { name: 'Run reviewed plan' });
  expect(request.mock.calls.some(([path]) => path.endsWith('/authoring/run'))).toBe(false); expect((screen.getByLabelText('Research') as HTMLInputElement).checked).toBe(true);
});
it('automatically recovers a lost Run reply before permitting another request and retains the canvas camera', async () => {
  const initial = drawing(), task = addCanvasTask(initial, session()); updateInstructions(initial, task, 'Useful work'); initial.camera = { x: 11, y: 29, zoom: .7 };
  const { state, request } = await mount(initial), original = request.getMockImplementation()!;
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => { const value = await original(path, body); if (path.endsWith('/authoring/run')) throw new Error('reply lost after acceptance'); return value as T; });
  await fireEvent.click(screen.getByRole('button', { name: 'Run pending plan' }));
  await waitFor(() => expect(request.mock.calls.some(([path]) => path.startsWith('/v1/workflow-requests/'))).toBe(true)); await waitFor(() => expect(state.pending).toBeNull());
  expect(request.mock.calls.filter(([path]) => path.endsWith('/authoring/run'))).toHaveLength(1); expect(state.camera).toEqual({ x: 11, y: 29, zoom: .7 });
});
it('clears an authoritatively refused Run while retaining saved work for a deliberate repair', async () => {
  const initial = drawing(), task = addCanvasTask(initial, session()); updateInstructions(initial, task, 'Keep this repairable work');
  const { state, request } = await mount(initial), original = request.getMockImplementation()!;
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => { if (path.endsWith('/authoring/run')) throw new DaemonError('invalid', 'Selected output is unavailable', 400); return original(path, body) as Promise<T>; });
  await fireEvent.click(screen.getByRole('button', { name: 'Run pending plan' }));
  await screen.findByText('Selected output is unavailable'); await waitFor(() => expect(state.pending).toBeNull());
  expect(state.busy).toBe(false); expect(state.tasks[0].definition.instructions).toBe('Keep this repairable work');
  expect(request.mock.calls.filter(([path]) => path.endsWith('/authoring/run'))).toHaveLength(1);
  expect(request.mock.calls.some(([path]) => path.startsWith('/v1/workflow-requests/'))).toBe(false);
  expect(screen.queryByRole('button', { name: 'Find submission receipt' })).toBeNull();
});
it('keeps the exact accepted Run recoverable when its follow-up saved-canvas read is lost', async () => {
  const initial = drawing(), task = addCanvasTask(initial, session()); updateInstructions(initial, task, 'Useful work');
  const { state, request } = await mount(initial), original = request.getMockImplementation()!;
  let loseRead = false;
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path.endsWith('/authoring') && loseRead) { loseRead = false; throw new Error('Follow-up read lost'); }
    const value = await original(path, body); if (path.endsWith('/authoring/run')) loseRead = true; return value as T;
  });
  await fireEvent.click(screen.getByRole('button', { name: 'Run pending plan' }));
  await waitFor(() => expect(request.mock.calls.some(([path]) => path.startsWith('/v1/workflow-requests/'))).toBe(true));
  await waitFor(() => expect(state.pending).toBeNull()); expect(state.tasks).toHaveLength(0);
  expect(request.mock.calls.filter(([path]) => path.endsWith('/authoring/run'))).toHaveLength(1);
});
it('keeps the last saved content and local candidate when a concurrent checkpoint wins', async () => {
  const { state, put, request } = await mount(); const original = request.getMockImplementation()!;
  put.mockRejectedValue(new DaemonError('conflict', 'Another client saved', 409));
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => path.endsWith('/authoring') ? { generation: 1, digest: 'remote', document: { ...canvasDocument(drawing()), sources: [{ id: 'remote-source', label: 'Their idea', value: { kind: 'text', text: 'Other client context' }, shared_name: null }] } satisfies AuthoringDocumentDto } as T : original(path, body) as Promise<T>);
  await add('My useful thought'); await screen.findByRole('button', { name: 'Use saved canvas' });
  expect(state.tasks[0].definition.instructions).toBe('My useful thought'); expect(state.authoringSave.status).toBe('conflict');
  await fireEvent.click(screen.getByRole('button', { name: 'Use saved canvas' })); expect(state.tasks).toHaveLength(0); expect(state.sources[0].label).toBe('Their idea');
});
it('loads topology without reading every task result, then fetches only the selected task', async () => {
  const a = acceptedTask('a'), b = acceptedTask('b'); b.definition.title = 'Second task';
  const { request } = await mount(drawing(), [a, b]);
  expect(request.mock.calls.filter(([path]) => path.includes('/tasks/'))).toEqual([]);
  await fireEvent.click(screen.getByRole('button', { name: 'Second task · queued' })); await screen.findByRole('heading', { name: 'Second task' });
  expect(request.mock.calls.filter(([path]) => path.includes('/tasks/')).map(([path]) => path)).toEqual(['/v1/workflows/workflow-a/tasks/b']);
});
it('discovers corrections and future consumers only on the explicit correction action', async () => {
  const failed = acceptedTask('failed'), corrected = acceptedTask('corrected'), future = acceptedTask('future');
  failed.state = 'failed'; failed.definition.title = 'Original work';
  corrected.state = 'completed'; corrected.definition.title = 'Successful repair'; corrected.definition.corrective_of = failed.id;
  future.definition.title = 'Future consumer'; future.definition.inputs = { context: { kind: 'task_output', task: failed.id, output: 'report', optional: false } };
  const { request, state } = await mount(drawing(), [failed, corrected, future]);
  expect(request.mock.calls.filter(([path]) => path.includes('/tasks/'))).toEqual([]);
  await fireEvent.click(screen.getByRole('button', { name: 'Original work · failed' })); await screen.findByRole('button', { name: 'Use correction for future work' });
  expect(request.mock.calls.filter(([path]) => path.includes('/tasks/'))).toHaveLength(1);
  const selected = state.selected, camera = copy(state.camera);
  await fireEvent.click(screen.getByRole('button', { name: 'Use correction for future work' }));
  await screen.findByRole('option', { name: 'Successful repair' }); await waitFor(() => expect((screen.getByLabelText('Successful correction') as HTMLSelectElement).disabled).toBe(false));
  expect(screen.getByRole('checkbox', { name: 'Future consumer' })).toBeTruthy();
  await fireEvent.change(screen.getByLabelText('Successful correction'), { target: { value: corrected.id } }); await fireEvent.click(screen.getByLabelText('Future consumer'));
  await fireEvent.click(screen.getByRole('button', { name: 'Use selected correction' }));
  expect(state.changes).toEqual([{ kind: 'adopt_correction', task: failed.id, correction: corrected.id, redirects: [{ consumer: future.id, expected_version: 1, input: 'context' }] }]);
  expect(state.selected).toBe(selected); expect(state.camera).toEqual(camera); expect(request.mock.calls.filter(([path]) => path.includes('/tasks/'))).toHaveLength(3);
});
it('connects to an accepted task through the list without requiring a prior task inspection', async () => {
  const task = acceptedTask(), initial = drawing(); addSource(initial, 'Useful context', { kind: 'text', text: 'Use this evidence' });
  const { state, request } = await mount(initial, [task]);
  await fireEvent.click(screen.getByRole('button', { name: 'List' }));
  const source = screen.getByRole('button', { name: /Useful context/ }).closest('article')!;
  await fireEvent.click(within(source).getByRole('button', { name: 'Connect to…' }));
  await fireEvent.change(screen.getByLabelText('Destination task'), { target: { value: task.id } });
  expect(request.mock.calls.filter(([path]) => path.includes('/tasks/'))).toHaveLength(0); expect(state.tasks).toHaveLength(0);
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(state.connections).toHaveLength(1)); expect(state.connections[0].target).toBe(task.id);
  expect(state.tasks[0]).toMatchObject({ id: task.id, version: 1 }); expect(state.error).toBe('');
  expect(request.mock.calls.filter(([path]) => path.includes('/tasks/'))).toHaveLength(1);
});
it('frees a full saved layout without changing current cards, pending work or authoring', async () => {
  const task = acceptedTask(), initial = drawing(), draft = addCanvasTask(initial, session()), source = addSource(initial, 'Pending evidence');
  initial.positions[task.id] = { x: 975, y: 321 };
  const preserved = copy(initial.positions), document = canvasDocument(initial);
  for (let index = 0; index < 597; index++) initial.positions[`old_${index}`] = { x: index, y: 999 };
  expect(Object.keys(initial.positions)).toHaveLength(600);
  const { state, put } = await mount(initial, [task]);
  await fireEvent.click(screen.getByRole('button', { name: '+ Add task' })); expect(state.tasks).toHaveLength(1);
  expect(screen.getByRole('alert').textContent).toContain('Remove off-page positions');
  await fireEvent.click(screen.getByRole('button', { name: 'Workflow' })); expect(screen.queryByRole('button', { name: 'Clear cached details' })).toBeNull();
  await fireEvent.click(screen.getByRole('button', { name: 'Remove off-page positions' }));
  expect(state.positions).toEqual(preserved); expect(state.positions[draft.id]).toEqual(preserved[draft.id]); expect(state.positions[source.id]).toEqual(preserved[source.id]);
  expect(canvasDocument(state)).toEqual(document);
  await screen.findByText('Saved', { selector: '.save-state' });
  await waitFor(() => expect(put.mock.calls.some(([path]) => path.endsWith('/positions'))).toBe(true));
  expect(put.mock.calls.filter(([path]) => path.endsWith('/authoring'))).toHaveLength(0);
  await fireEvent.click(screen.getByRole('button', { name: '+ Add task' })); expect(state.tasks).toHaveLength(2);
});
it('saves off-page reference cleanup through authoring and positions without executing work', async () => {
  const task = acceptedTask(), initial = drawing(), source = addSource(initial, 'Old connected input'), idea = addSource(initial, 'Unconnected idea');
  initial.accepted.old_card = 'off-page-task'; initial.positions['off-page-task'] = { x: 800, y: 800 };
  initial.connections = [{ id: 'old-wire', source: source.id, target: 'old_card', input: 'context', output: null, kind: 'data', optional: false, outcome: false }];
  const { state, put, request, saved } = await mount(initial, [task]);
  await fireEvent.click(screen.getByRole('button', { name: 'Workflow' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Remove off-page canvas references' }));
  await waitFor(() => expect(put.mock.calls.some(([path]) => path.endsWith('/authoring'))).toBe(true));
  await waitFor(() => expect(put.mock.calls.some(([path]) => path.endsWith('/positions'))).toBe(true));
  await screen.findByText('Saved', { selector: '.save-state' });
  expect(saved().connections).toEqual([]); expect(saved().accepted).toEqual({}); expect(saved().sources.map((item) => item.id)).toEqual([idea.id]);
  expect(state.positions[source.id]).toBeUndefined(); expect(state.positions['off-page-task']).toBeUndefined(); expect(state.positions[idea.id]).toBeDefined();
  expect(request.mock.calls.filter(([, body]) => body)).toEqual([]);
  expect(screen.queryByRole('button', { name: 'Remove off-page canvas references' })).toBeNull();
});
it('preserves selected input while supplied content waits for an uncertain control receipt', async () => {
  const task = acceptedTask(); task.definition.inputs = { source: { kind: 'supplied', name: 'topic', optional: false } }; const initial = drawing(), source = addSource(initial, 'Topic', null); source.sharedName = 'topic';
  const { state, request } = await mount(initial, [task]); const original = request.getMockImplementation()!;
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => { if (path.endsWith('/changes')) throw new Error('reply lost'); return original(path, body) as Promise<T>; });
  await fireEvent.click(screen.getByRole('button', { name: 'Pause workflow' })); await screen.findByRole('button', { name: 'Find submission receipt' });
  expect((screen.getByRole('button', { name: '+ Text' }) as HTMLButtonElement).disabled).toBe(true); expect(state.sources[0].sharedName).toBe('topic');
  await fireEvent.click(screen.getByRole('button', { name: 'Find submission receipt' })); await waitFor(() => expect(state.pending).toBeNull()); await waitFor(() => expect((screen.getByRole('button', { name: '+ Text' }) as HTMLButtonElement).disabled).toBe(false));
});

it('does not label a layout saved while its independent write is pending or refused', async () => {
  const initial = drawing(); addCanvasTask(initial, session());
  const { state, put, request } = await mount(initial), originalPut = put.getMockImplementation()!, originalRead = request.getMockImplementation()!;
  const savedPositions = copy(state.positions); let reject!: (failure: Error) => void;
  const pending = new Promise<never>((_, refusal) => { reject = refusal; });
  put.mockImplementation(async <T>(path: string, body: unknown): Promise<T> => path.endsWith('/positions') ? pending : originalPut(path, body) as Promise<T>);
  request.mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => path.endsWith('/positions') ? { generation: 0, digest: 'positions', positions: savedPositions } as T : originalRead(path, body) as Promise<T>);
  await fireEvent.keyDown(screen.getByRole('button', { name: 'Move New task' }), { key: 'ArrowRight' });
  await screen.findByText('Saving…', { selector: '.save-state' }); expect(state.authoringSave.status).toBe('saved');
  reject(new Error('Position write refused')); await screen.findByRole('button', { name: 'Retry position save' });
  expect(screen.getByText('Not saved', { selector: '.save-state' })).toBeTruthy(); expect(state.positions).not.toEqual(savedPositions);
  put.mockImplementation(originalPut); await fireEvent.click(screen.getByRole('button', { name: 'Retry position save' })); await screen.findByText('Saved', { selector: '.save-state' });
});
