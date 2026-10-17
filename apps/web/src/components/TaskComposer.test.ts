import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import TaskComposer from './TaskComposer.svelte';
import fixture from '../../../../crates/control-protocol/tests/fixtures/task-session.json';
import { DaemonClient } from '../lib/transport';

afterEach(cleanup);

it('separates task instructions from supplied context and execution choices', () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  render(TaskComposer, { client, session: fixture.value, oncreated: vi.fn(async () => {}), oncancel: vi.fn(), onerror: vi.fn() });
  expect(within(screen.getByRole('group', { name: 'Task instructions' })).getByLabelText('Instructions')).toBeTruthy();
  const context = within(screen.getByRole('group', { name: 'Supplied context' }));
  expect(context.getByLabelText('Brief input')).toBeTruthy();
  expect(context.getByLabelText('Text file input')).toBeTruthy();
  expect(context.queryByLabelText('Instructions')).toBeNull();
  expect(within(screen.getByRole('group', { name: 'Execution' })).getByLabelText('Capability')).toBeTruthy();
});

it('retains and replays the exact submission after an unknown reply', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  const request = vi.spyOn(client, 'request')
    .mockRejectedValueOnce(new Error('Reply was lost'))
    .mockResolvedValueOnce({ request_id: 'accepted', workflow_id: 'workflow-a', generation: 1, aliases: { task: 'task-a' } });
  const created = vi.fn(async () => {});
  render(TaskComposer, { client, session: fixture.value, oncreated: created, oncancel: vi.fn(), onerror: vi.fn() });
  await fireEvent.input(screen.getByLabelText('Task title'), { target: { value: 'Inspect Cargo project' } });
  await fireEvent.input(screen.getByLabelText('Instructions'), { target: { value: 'Report its dependencies' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Submit task' }).closest('form')!);
  expect(screen.getByRole('alert').textContent).toContain('Reply was lost');
  expect(screen.getByRole('button', { name: 'Find submission receipt' })).toBeTruthy();
  expect(created).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Replay exact submission' }));
  expect(request.mock.calls[1]).toEqual(request.mock.calls[0]);
  expect(created).toHaveBeenCalledTimes(1);
});

it('declares a required input without inventing its value when it is deferred', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  const request = vi.spyOn(client, 'request').mockResolvedValue({ request_id: 'accepted', workflow_id: 'workflow-a', generation: 1, aliases: { task: 'task-a' } });
  render(TaskComposer, { client, session: fixture.value, oncreated: vi.fn(async () => {}), oncancel: vi.fn(), onerror: vi.fn() });
  await fireEvent.click(screen.getByLabelText('Supply brief now'));
  await fireEvent.submit(screen.getByRole('button', { name: 'Submit task' }).closest('form')!);
  expect(request.mock.calls[0]?.[1]).toMatchObject({ inputs: {}, tasks: [{ definition: { inputs: { brief: { kind: 'supplied', name: 'brief', optional: false } } } }] });
});

it('uses the connected daemon input limit before reading or uploading a file', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  const request = vi.spyOn(client, 'request');
  render(TaskComposer, { client, session: { ...fixture.value, input_limit_bytes: 8 }, oncreated: vi.fn(async () => {}), oncancel: vi.fn(), onerror: vi.fn() });
  await fireEvent.change(screen.getByLabelText('Text file input'), { target: { files: [new File(['ninebytes'], 'large.txt', { type: 'text/plain' })] } });
  expect(screen.getByRole('alert').textContent).toContain('daemon upload bound of 8 bytes');
  expect(request).not.toHaveBeenCalled();
});

it('sends only the explicit model continuation and tool choices in the initial request', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  const request = vi.spyOn(client, 'request').mockResolvedValue({ request_id: 'accepted', workflow_id: 'workflow-a', generation: 1, aliases: { task: 'task-a' } });
  render(TaskComposer, { client, session: { ...fixture.value, capabilities: [{ id: 'model', operation: 'model.generate' }] }, oncreated: vi.fn(async () => {}), oncancel: vi.fn(), onerror: vi.fn() });
  await fireEvent.click(screen.getByLabelText('Finish when required work settles'));
  await fireEvent.click(screen.getByLabelText('Allow model to propose future work'));
  await fireEvent.click(screen.getByLabelText('Allow automatic feedback after refused proposals'));
  await fireEvent.click(screen.getByLabelText('Permit bounded file reads in declared paths'));
  await fireEvent.input(screen.getByLabelText('Absolute project root'), { target: { value: '/project/research' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Submit task' }).closest('form')!);
  expect(request.mock.calls[0]?.[1]).toMatchObject({ finish_when_settled: false, tasks: [{ definition: {
    actions: { automatic_repair: true }, tools: [{ kind: 'read_file' }], paths: [{ path: '/project/research', write: false }],
  } }] });
});

it('hides model options for processes and clears model choices when switching capability kind', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  const request = vi.spyOn(client, 'request').mockResolvedValue({ request_id: 'accepted', workflow_id: 'workflow-a', generation: 1, aliases: { task: 'task-a' } });
  const capabilities = [{ id: 'process', operation: 'process.execute' }, { id: 'model', operation: 'model.generate' }, { id: 'another-model', operation: 'model.generate' }];
  render(TaskComposer, { client, session: { ...fixture.value, capabilities }, oncreated: vi.fn(async () => {}), oncancel: vi.fn(), onerror: vi.fn() });
  expect(screen.queryByLabelText('Allow model to propose future work')).toBeNull();
  await fireEvent.change(screen.getByLabelText('Capability'), { target: { value: '1' } });
  await fireEvent.click(screen.getByLabelText('Allow model to propose future work'));
  await fireEvent.click(screen.getByLabelText('Allow automatic feedback after refused proposals'));
  await fireEvent.click(screen.getByLabelText('Permit bounded file reads in declared paths'));
  await fireEvent.change(screen.getByLabelText('Capability'), { target: { value: '2' } });
  expect((screen.getByLabelText('Allow automatic feedback after refused proposals') as HTMLInputElement).checked).toBe(true);
  expect((screen.getByLabelText('Permit bounded file reads in declared paths') as HTMLInputElement).checked).toBe(true);
  await fireEvent.change(screen.getByLabelText('Capability'), { target: { value: '0' } });
  expect(screen.queryByLabelText('Allow model to propose future work')).toBeNull();
  await fireEvent.submit(screen.getByRole('button', { name: 'Submit task' }).closest('form')!);
  expect(request.mock.calls[0]?.[1]).toMatchObject({ tasks: [{ definition: { capability: capabilities[0], actions: null, tools: [] } }] });
});
