import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import TaskComposerTestHarness from './TaskComposerTestHarness.svelte';
import fixture from '../../../../crates/control-protocol/tests/fixtures/task-session.json';
import { DaemonClient, DaemonError } from '../lib/transport';
import { capabilityKey, createTaskDraft, type TaskDraft, type TaskDraftTarget } from '../lib/task-composer';
import type { TaskSessionDto, WorkflowReceipt } from '../lib/protocol.generated';

afterEach(cleanup);
const receipt: WorkflowReceipt = { request_id: 'accepted', workflow_id: 'workflow-a', generation: 8, aliases: { task: 'task-a' } };
const session = { ...fixture.value, capabilities: [{ id: 'process', operation: 'process.execute' }] };
function mount(target: TaskDraftTarget = { kind: 'new' }, connected: TaskSessionDto = session) {
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  const accepted = vi.fn(async () => {});
  const props = { client, session: connected, initial: createTaskDraft(connected, target), onaccepted: accepted };
  return { client, accepted, props, view: render(TaskComposerTestHarness, props) };
}
const draft = (): TaskDraft => JSON.parse(screen.getByTestId('task-draft').textContent!) as TaskDraft;
async function write(instructions = 'Inspect the Cargo project') {
  await fireEvent.input(screen.getByLabelText('What should this task do?'), { target: { value: instructions } });
}
async function submit(name = 'Create workflow') {
  await fireEvent.submit(screen.getByRole('button', { name }).closest('form')!);
}

it('creates from instructions alone with a derived editable title and no invented brief', async () => {
  const { client } = mount();
  const request = vi.spyOn(client, 'request').mockResolvedValue(receipt);
  expect(screen.queryByLabelText('Context')).toBeNull();
  await write();
  expect((screen.getByLabelText('Task title') as HTMLInputElement).value).toBe('Inspect the Cargo project');
  await fireEvent.input(screen.getByLabelText('Task title'), { target: { value: 'My task title' } });
  await write('New instructions');
  expect((screen.getByLabelText('Task title') as HTMLInputElement).value).toBe('My task title');
  await submit();
  expect(request.mock.calls[0]?.[1]).toMatchObject({ title: 'My task title', inputs: {}, limits: null, finish_when_settled: true, tasks: [{ definition: { title: 'My task title', instructions: 'New instructions', inputs: {}, paths: [], actions: null, tools: [] } }] });
});

it('retains and replays an exact target and request across composer unmounts', async () => {
  const { client, accepted, view, props } = mount({ kind: 'existing', workflowId: 'workflow-a', title: 'Workflow A', generation: 7 });
  const request = vi.spyOn(client, 'request').mockRejectedValueOnce(new Error('Reply was lost')).mockResolvedValueOnce(receipt);
  await write();
  await submit('Add task');
  expect(screen.getByRole('alert').textContent).toContain('Reply was lost');
  await view.rerender({ ...props, visible: false });
  await view.rerender({ ...props, visible: true });
  expect(accepted).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Replay exact submission' }));
  expect(request.mock.calls[1]).toEqual(request.mock.calls[0]);
  expect(request.mock.calls[0]).toEqual(['/v1/workflows/workflow-a/changes', expect.objectContaining({ expected_generation: 7, changes: [{ kind: 'add', alias: 'task', definition: expect.objectContaining({ instructions: 'Inspect the Cargo project' }) }] })]);
  expect(accepted).toHaveBeenCalledTimes(1);
});

it('keeps deliberate missing context local to its task without binding shared workflow inputs', async () => {
  const { client } = mount({ kind: 'existing', workflowId: 'workflow-a', title: 'A', generation: 7 });
  const request = vi.spyOn(client, 'request').mockResolvedValue(receipt);
  await write();
  await fireEvent.click(screen.getByRole('button', { name: 'Add context' }));
  await fireEvent.click(screen.getByLabelText('Supply context now'));
  await submit('Add task');
  expect(request.mock.calls[0]?.[1]).toMatchObject({ changes: [{ kind: 'add', definition: { inputs: { brief: { kind: 'supplied', name: expect.stringMatching(/^context_[a-z0-9]+$/), optional: false } } } }] });
  expect((request.mock.calls[0]?.[1] as { changes: unknown[] }).changes).toHaveLength(1);
});

it('supplies distinct optional context inline and retains text when hidden', async () => {
  const { client, view, props } = mount();
  const request = vi.spyOn(client, 'request').mockResolvedValue(receipt);
  await write();
  await fireEvent.click(screen.getByRole('button', { name: 'Add context' }));
  await fireEvent.input(screen.getByLabelText('Context'), { target: { value: 'Only this task receives this text' } });
  await view.rerender({ ...props, visible: false });
  await view.rerender({ ...props, visible: true });
  expect((screen.getByLabelText('Context') as HTMLTextAreaElement).value).toBe('Only this task receives this text');
  await submit();
  expect(request.mock.calls[0]?.[1]).toMatchObject({ inputs: {}, tasks: [{ definition: { inputs: { brief: { kind: 'inline', value: { kind: 'text', text: 'Only this task receives this text' } } } } }] });
});

it('checks the connected daemon upload limit before reading or uploading a file', async () => {
  const { client } = mount({ kind: 'new' }, { ...session, input_limit_bytes: 8 });
  const request = vi.spyOn(client, 'request');
  await fireEvent.change(screen.getByLabelText('Attach text file'), { target: { files: [new File(['ninebytes'], 'large.txt', { type: 'text/plain' })] } });
  expect(screen.getByRole('alert').textContent).toContain('daemon upload bound of 8 bytes');
  expect(request).not.toHaveBeenCalled();
});

it('requires an intentional executor choice and retains its stable identity across catalog reorder', async () => {
  const capabilities = [{ id: 'process', operation: 'process.execute' }, { id: 'model', operation: 'model.generate' }];
  const { client, view, props } = mount({ kind: 'new' }, { ...session, capabilities });
  const request = vi.spyOn(client, 'request').mockResolvedValue(receipt);
  await write();
  expect((screen.getByRole('button', { name: 'Create workflow' }) as HTMLButtonElement).disabled).toBe(true);
  await fireEvent.change(screen.getByLabelText('Executor'), { target: { value: capabilityKey(capabilities[1]!) } });
  await view.rerender({ ...props, session: { ...session, capabilities: [...capabilities].reverse() } });
  await submit();
  expect(request.mock.calls[0]?.[1]).toMatchObject({ tasks: [{ definition: { capability: capabilities[1] } }] });
});

it('clears model continuation and tool choices only when choosing a non-model executor', async () => {
  const capabilities = [{ id: 'process', operation: 'process.execute' }, { id: 'model', operation: 'model.generate' }, { id: 'another-model', operation: 'model.generate' }];
  const { client } = mount({ kind: 'new' }, { ...session, capabilities });
  const request = vi.spyOn(client, 'request').mockResolvedValue(receipt);
  await write();
  await fireEvent.change(screen.getByLabelText('Executor'), { target: { value: capabilityKey(capabilities[1]!) } });
  await fireEvent.click(screen.getByLabelText('Allow model to propose future work'));
  await fireEvent.click(screen.getByLabelText('Allow automatic feedback after refused proposals'));
  await fireEvent.click(screen.getByLabelText('Permit bounded file reads in declared paths'));
  await fireEvent.change(screen.getByLabelText('Executor'), { target: { value: capabilityKey(capabilities[2]!) } });
  expect((screen.getByLabelText('Allow automatic feedback after refused proposals') as HTMLInputElement).checked).toBe(true);
  await fireEvent.change(screen.getByLabelText('Executor'), { target: { value: capabilityKey(capabilities[0]!) } });
  await submit();
  expect(request.mock.calls[0]?.[1]).toMatchObject({ tasks: [{ definition: { capability: capabilities[0], actions: null, tools: [] } }] });
});

it('retains a stale-generation draft and requires explicit refresh then a new submission', async () => {
  const { client } = mount({ kind: 'existing', workflowId: 'workflow-a', title: 'A', generation: 7 });
  const request = vi.spyOn(client, 'request').mockRejectedValueOnce(new DaemonError('conflict', 'Workflow changed', 409)).mockResolvedValueOnce({ generation: 9 }).mockResolvedValueOnce(receipt);
  await write();
  await submit('Add task');
  expect((screen.getByRole('button', { name: 'Add task' }) as HTMLButtonElement).disabled).toBe(true);
  await fireEvent.click(screen.getByRole('button', { name: 'Refresh workflow for review' }));
  expect(request).toHaveBeenCalledTimes(2);
  expect(draft().definition.instructions).toBe('Inspect the Cargo project');
  await submit('Add task');
  expect(request.mock.calls[2]?.[1]).toMatchObject({ expected_generation: 9 });
  expect((request.mock.calls[2]?.[1] as { request_id: string }).request_id).not.toBe((request.mock.calls[0]?.[1] as { request_id: string }).request_id);
});

it('prevents double submissions and ignores a late result after authority disposal', async () => {
  const { client, accepted, view, props } = mount();
  let resolve!: (value: WorkflowReceipt) => void;
  const request = vi.spyOn(client, 'request').mockImplementation(() => new Promise((done) => { resolve = done as (value: WorkflowReceipt) => void; }));
  await write();
  await submit();
  await fireEvent.submit(screen.getByRole('button', { name: 'Replay exact submission' }).closest('form')!);
  expect(request).toHaveBeenCalledTimes(1);
  await view.rerender({ ...props, disposed: true });
  resolve(receipt);
  await waitFor(() => expect(draft().disposed).toBe(true));
  expect(accepted).not.toHaveBeenCalled();
  expect(draft().accepted).toBe(false);
});

it('retains explicit model policy, paths and unfinished advanced fields across navigation', async () => {
  const modelSession = { ...session, capabilities: [{ id: 'model', operation: 'model.generate' }] };
  const { client, view, props } = mount({ kind: 'new' }, modelSession);
  const request = vi.spyOn(client, 'request').mockResolvedValue(receipt);
  await write();
  await fireEvent.click(screen.getByLabelText('Finish when required work settles'));
  await fireEvent.click(screen.getByLabelText('Allow model to propose future work'));
  await fireEvent.click(screen.getByLabelText('Allow automatic feedback after refused proposals'));
  await fireEvent.click(screen.getByLabelText('Permit bounded file reads in declared paths'));
  await fireEvent.input(screen.getByLabelText('Project path'), { target: { value: '/project/research' } });
  await view.rerender({ ...props, visible: false });
  await view.rerender({ ...props, visible: true });
  expect((screen.getByLabelText('Project path') as HTMLInputElement).value).toBe('/project/research');
  await fireEvent.click(screen.getByRole('button', { name: 'Add path' }));
  await submit();
  expect(request.mock.calls[0]?.[1]).toMatchObject({ finish_when_settled: false, tasks: [{ definition: { actions: { automatic_repair: true }, tools: [{ kind: 'read_file' }], paths: [{ path: '/project/research', write: false }] } }] });
});

it('recovers a captured attachment with the same upload and binds only this task', async () => {
  const { client } = mount({ kind: 'existing', workflowId: 'workflow-a', title: 'A', generation: 7 });
  const request = vi.spyOn(client, 'request').mockRejectedValueOnce(new Error('Upload reply lost')).mockResolvedValueOnce({ input_id: 'upload-a', name: 'brief.txt', bytes: 4, digest: 'digest-a' }).mockResolvedValueOnce(receipt);
  const file = new File(['text'], 'brief.txt', { type: 'text/plain' });
  Object.defineProperty(file, 'arrayBuffer', { value: async () => new TextEncoder().encode('text').buffer });
  await fireEvent.change(screen.getByLabelText('Attach text file'), { target: { files: [file] } });
  await fireEvent.click(screen.getByRole('button', { name: 'Retry exact upload' }));
  expect(request.mock.calls[1]).toEqual(request.mock.calls[0]);
  await write();
  await submit('Add task');
  expect(request.mock.calls[2]?.[1]).toMatchObject({ changes: [{ kind: 'add', definition: { inputs: { file: { kind: 'inline', value: { kind: 'uploaded', input_id: 'upload-a' } } } } }] });
});

it('looks up the retained request receipt without issuing another mutation', async () => {
  const { client, accepted } = mount();
  const request = vi.spyOn(client, 'request').mockRejectedValueOnce(new Error('Lost reply')).mockResolvedValueOnce(receipt);
  await write();
  await submit();
  const pendingId = draft().pending!.request.request_id;
  await fireEvent.click(screen.getByRole('button', { name: 'Find submission receipt' }));
  expect(request.mock.calls[1]).toEqual([`/v1/workflow-requests/${pendingId}`]);
  expect(accepted).toHaveBeenCalledWith(receipt);
});

it('keeps an authored draft after a known refusal without treating it as an uncertain reply', async () => {
  const { client } = mount();
  const request = vi.spyOn(client, 'request').mockRejectedValueOnce(new DaemonError('refused', 'Not permitted', 400));
  await write();
  await submit();
  expect(draft().pending).toBeNull();
  expect(draft().definition.instructions).toBe('Inspect the Cargo project');
  expect(screen.queryByRole('button', { name: 'Replay exact submission' })).toBeNull();
  expect(request).toHaveBeenCalledTimes(1);
});
