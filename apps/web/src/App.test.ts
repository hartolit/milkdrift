import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { DaemonClient } from './lib/transport';
import type { StreamEvent } from './lib/transport';
import fixture from '../../../crates/control-protocol/tests/fixtures/task-session.json';

afterEach(() => { cleanup(); vi.restoreAllMocks(); window.localStorage.clear(); });

it('keeps the selected discovery page during observations and resets its cursor for a submitted search', async () => {
  let emit: (event: StreamEvent) => Promise<void> = async () => { throw new Error('stream not connected'); };
  vi.spyOn(DaemonClient.prototype, 'stream').mockImplementation(async (_path, onEvent, signal) => {
    emit = onEvent;
    await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  });
  const request = vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(async <T>(path: string): Promise<T> => {
    if (path === '/v1/session') return fixture.value as T;
    const url = new URL(path, 'http://127.0.0.1');
    const second = url.searchParams.get('cursor') === 'page-two';
    return { items: [{ id: second ? 'second' : 'first', title: second ? 'Second page task' : 'First page task', description: 'Retained work', generation: 1, state: 'waiting', created_at: 1, updated_at: 1, task_count: 1 }], next_cursor: second ? null : 'page-two', truncated: !second } as T;
  });
  render(App);
  await fireEvent.click(screen.getByRole('button', { name: 'Connect a daemon' }));
  await fireEvent.input(screen.getByLabelText('Connection name'), { target: { value: 'Pages' } });
  await fireEvent.input(screen.getByLabelText('Daemon URL'), { target: { value: 'http://127.0.0.1:10001' } });
  await fireEvent.input(screen.getByLabelText('Installation identity'), { target: { value: fixture.value.daemon_id } });
  await fireEvent.input(screen.getByLabelText('Bearer credential'), { target: { value: 'private-test-credential' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(screen.getByText('First page task')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: 'Next page' }));
  await waitFor(() => expect(screen.getByText('Second page task')).toBeTruthy());
  await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'new search' } });
  await emit({ event: 'workflow_changed', id: 'stream-cursor', data: '{}' });
  expect(request.mock.calls.at(-1)?.[0]).toBe('/v1/workflows?limit=50&search=&cursor=page-two');
  expect(screen.getByText('Second page task')).toBeTruthy();
  await fireEvent.submit(screen.getByRole('searchbox').closest('form')!);
  await waitFor(() => expect(request.mock.calls.at(-1)?.[0]).toBe('/v1/workflows?limit=50&search=new+search'));
  await waitFor(() => expect(screen.getByText('First page task')).toBeTruthy());
});

it('keeps another daemon draft open when an earlier connection confirms a submission', async () => {
  vi.spyOn(DaemonClient.prototype, 'stream').mockImplementation(async (_path, _onEvent, signal) => {
    await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  });
  let accept!: (receipt: unknown) => void;
  const request = vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(async function <T>(this: DaemonClient, path: string, body?: unknown): Promise<T> {
    if (path === '/v1/session') return { ...fixture.value, daemon_id: this.endpoint.endsWith('10001') ? 'daemon-a' : 'daemon-b' } as T;
    if (path === '/v1/workflows' && body) return new Promise<T>((resolve) => { accept = resolve as (receipt: unknown) => void; });
    if (path === '/v1/workflows/workflow-a') return { id: 'workflow-a', tasks: [] } as T;
    return { items: [], next_cursor: null, truncated: false } as T;
  });
  render(App);
  for (const [name, port] of [['a', '10001'], ['b', '10002']]) {
    await fireEvent.click(screen.getByRole('button', { name: 'Add daemon connection' }));
    await fireEvent.input(screen.getByLabelText('Connection name'), { target: { value: `Studio ${name}` } });
    await fireEvent.input(screen.getByLabelText('Daemon URL'), { target: { value: `http://127.0.0.1:${port}` } });
    await fireEvent.input(screen.getByLabelText('Installation identity'), { target: { value: `daemon-${name}` } });
    await fireEvent.input(screen.getByLabelText('Bearer credential'), { target: { value: `private-${name}` } });
    await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    await screen.findByRole('heading', { name: 'Your workflows' });
    await fireEvent.click(screen.getByRole('button', { name: '+ New task' }));
    await fireEvent.input(screen.getByLabelText('Task title'), { target: { value: name === 'a' ? 'First task' : 'Second daemon draft' } });
    if (name === 'a') await fireEvent.submit(screen.getByRole('button', { name: 'Submit task' }).closest('form')!);
  }
  accept({ request_id: 'first-request', workflow_id: 'workflow-a', generation: 1, aliases: {} });
  await waitFor(() => expect(request.mock.calls.some(([path]) => path === '/v1/workflows/workflow-a')).toBe(true));
  expect((screen.getByLabelText('Task title') as HTMLInputElement).value).toBe('Second daemon draft');
});

it('opens the accepted workflow from its receipt while discovery refresh remains pending', async () => {
  vi.spyOn(DaemonClient.prototype, 'stream').mockImplementation(async (_path, _onEvent, signal) => {
    await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  });
  let listReads = 0;
  let releaseList!: (value: unknown) => void;
  const delayedList = new Promise((resolve) => { releaseList = resolve; });
  const emptyPage = { items: [], next_cursor: null, truncated: false };
  const request = vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/session') return fixture.value as T;
    if (path === '/v1/workflows' && body) return { request_id: 'accepted-request', workflow_id: 'accepted-workflow', generation: 1, aliases: {} } as T;
    if (path === '/v1/workflows/accepted-workflow') return {
      id: 'accepted-workflow', title: 'Accepted model work', description: 'Continues in the daemon',
      generation: 1, state: 'waiting', created_at: 1, updated_at: 1, task_count: 0, tasks: [],
      admissions: [], next_task_cursor: null, wait_question: null, limits: fixture.value.limits,
      admitted_tasks: 0, admitted_model_batches: 0, rejected_responses: 0, active_execution_ms: 0,
    } as T;
    if (path.startsWith('/v1/workflows?')) return (listReads++ === 0 ? emptyPage : await delayedList) as T;
    throw new Error(`Unexpected request ${path}`);
  });
  render(App);
  await fireEvent.click(screen.getByRole('button', { name: 'Add daemon connection' }));
  await fireEvent.input(screen.getByLabelText('Connection name'), { target: { value: 'Receipt studio' } });
  await fireEvent.input(screen.getByLabelText('Daemon URL'), { target: { value: 'http://127.0.0.1:10001' } });
  await fireEvent.input(screen.getByLabelText('Installation identity'), { target: { value: fixture.value.daemon_id } });
  await fireEvent.input(screen.getByLabelText('Bearer credential'), { target: { value: 'private-test-credential' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  await screen.findByRole('heading', { name: 'Your workflows' });
  await fireEvent.click(screen.getByRole('button', { name: '+ New task' }));
  await fireEvent.input(screen.getByLabelText('Task title'), { target: { value: 'Accepted model work' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Submit task' }).closest('form')!);
  try {
    await screen.findByRole('heading', { name: 'Accepted model work', level: 1 });
    expect(listReads).toBe(2);
    expect(request.mock.calls.filter(([path, body]) => path === '/v1/workflows' && body)).toHaveLength(1);
  } finally { releaseList(emptyPage); }
});
