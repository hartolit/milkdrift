import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { DaemonClient, DaemonError } from './lib/transport';
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
  await fireEvent.input(screen.getByLabelText('Daemon URL'), { target: { value: 'http://127.0.0.1:10001' } });
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
    await fireEvent.input(screen.getByLabelText('Daemon URL'), { target: { value: `http://127.0.0.1:${port}` } });
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
  await fireEvent.input(screen.getByLabelText('Daemon URL'), { target: { value: 'http://127.0.0.1:10001' } });
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

async function authenticate(endpoint = 'http://127.0.0.1:10001') {
  await fireEvent.input(screen.getByLabelText('Daemon URL'), { target: { value: endpoint } });
  await fireEvent.input(screen.getByLabelText('Bearer credential'), { target: { value: 'private-test-credential' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
}

function idleStream() {
  return vi.spyOn(DaemonClient.prototype, 'stream').mockImplementation(async (_path, _onEvent, signal) => {
    await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  });
}

it('adopts a legacy pinned profile, learns the daemon name, and reports a separate cache-write failure', async () => {
  window.localStorage.setItem('milkdrift.connections.v1', JSON.stringify([{ id: 'old-selection', label: 'Old nickname', endpoint: 'http://127.0.0.1:10001', daemonId: fixture.value.daemon_id }]));
  idleStream();
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('Storage unavailable'); });
  const request = vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(async <T>(path: string): Promise<T> =>
    (path === '/v1/session' ? { ...fixture.value, daemon_name: 'Shared studio', daemon_name_generation: 5 } : { items: [], next_cursor: null, truncated: false }) as T);
  const bind = vi.spyOn(DaemonClient.prototype, 'bindIdentity');
  render(App);
  expect(screen.getByRole('button', { name: /^Old nickname/ }).textContent).toContain('Cached name');
  await fireEvent.click(screen.getByRole('button', { name: /^Old nickname/ }));
  await authenticate();
  await screen.findByRole('heading', { name: 'Your workflows' });
  expect(screen.getByRole('button', { name: /^Shared studio/ })).toBeTruthy();
  expect(screen.queryByRole('button', { name: /^Old nickname/ })).toBeNull();
  expect(screen.getByRole('alert').textContent).toContain('could not save the connection cache');
  expect(bind).toHaveBeenCalledWith(fixture.value.daemon_id);
  expect(request.mock.calls.every(([, body]) => body === undefined)).toBe(true);
});

it('refuses a changed installation without discovery or overwriting the saved pin', async () => {
  const profile = { id: 'old-selection', label: 'Old nickname', endpoint: 'http://127.0.0.1:10001', daemonId: 'old-pin' };
  window.localStorage.setItem('milkdrift.connections.v1', JSON.stringify([profile]));
  const request = vi.spyOn(DaemonClient.prototype, 'request').mockResolvedValue(fixture.value);
  render(App);
  await fireEvent.click(screen.getByRole('button', { name: /^Old nickname/ }));
  await authenticate();
  expect(screen.getByRole('alert').textContent).toContain('different daemon');
  expect(screen.getByRole('alert').textContent).toContain('Forget this saved connection');
  expect(request).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole('heading', { name: 'Your workflows' })).toBeNull();
  expect(JSON.parse(window.localStorage.getItem('milkdrift.connections.v1')!)).toEqual([profile]);
  await fireEvent.click(screen.getByRole('button', { name: 'Forget Old nickname connection' }));
  expect(window.localStorage.getItem('milkdrift.connections.v1')).toBe('[]');
});

it('discards an interrupted bootstrap even if its transport resolves after cancellation', async () => {
  let resolve!: (session: unknown) => void;
  const request = vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(() => new Promise((done) => { resolve = done; }));
  render(App);
  await fireEvent.click(screen.getByRole('button', { name: 'Connect a daemon' }));
  await authenticate();
  await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  resolve(fixture.value);
  await Promise.resolve();
  expect(request).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole('heading', { name: 'Your workflows' })).toBeNull();
  expect(window.localStorage.length).toBe(0);
});

it('refreshes the shared name when workflow discovery is forbidden, then ignores a late name after disconnect', async () => {
  let emit!: (event: StreamEvent) => Promise<void>;
  vi.spyOn(DaemonClient.prototype, 'stream').mockImplementation(async (_path, onEvent, signal) => {
    emit = onEvent;
    await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  });
  let name = 'Studio A';
  let resolve!: (session: unknown) => void;
  let delay = false;
  const request = vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(async <T>(path: string): Promise<T> => {
    if (path === '/v1/session') return (delay ? await new Promise<unknown>((done) => { resolve = done; }) : { ...fixture.value, daemon_name: name, daemon_name_generation: name === 'Studio A' ? 1 : 2 }) as T;
    throw new DaemonError('forbidden', 'Workflow discovery is not permitted.', 403);
  });
  render(App);
  await fireEvent.click(screen.getByRole('button', { name: 'Connect a daemon' }));
  await authenticate();
  await screen.findByRole('heading', { name: 'Your workflows' });
  expect(screen.getByRole('button', { name: /^Studio A/ })).toBeTruthy();
  name = 'Shared new name';
  await emit({ event: 'workflow_changed', data: '{}' });
  await waitFor(() => expect(screen.getByRole('button', { name: /^Shared new name/ })).toBeTruthy());
  delay = true;
  const observed = emit({ event: 'workflow_changed', data: '{}' });
  await waitFor(() => expect(request.mock.calls.filter(([path]) => path === '/v1/session')).toHaveLength(3));
  await fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }));
  resolve({ ...fixture.value, daemon_name: 'Stale late name', daemon_name_generation: 3 });
  await observed;
  expect(screen.queryByRole('button', { name: /^Stale late name/ })).toBeNull();
  expect(screen.getByRole('button', { name: /^Shared new name/ }).textContent).toContain('Cached name');
  expect(window.localStorage.getItem('milkdrift.connections.v1')).not.toContain('Stale late name');
});

it('applies a name refresh while a workflow read is still pending', async () => {
  let emit!: (event: StreamEvent) => Promise<void>;
  vi.spyOn(DaemonClient.prototype, 'stream').mockImplementation(async (_path, onEvent, signal) => {
    emit = onEvent;
    await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  });
  let name = 'Original';
  let finish!: (value: unknown) => void;
  let holdDiscovery = false;
  const emptyPage = { items: [], next_cursor: null, truncated: false };
  vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(async <T>(path: string): Promise<T> => {
    if (path === '/v1/session') return { ...fixture.value, daemon_name: name, daemon_name_generation: name === 'Original' ? 1 : 2 } as T;
    return (holdDiscovery ? await new Promise<unknown>((resolve) => { finish = resolve; }) : emptyPage) as T;
  });
  render(App);
  await fireEvent.click(screen.getByRole('button', { name: 'Connect a daemon' }));
  await authenticate();
  name = 'Shared while discovery waits';
  holdDiscovery = true;
  const observation = emit({ event: 'workflow_changed', data: '{}' });
  await screen.findByRole('button', { name: /^Shared while discovery waits/ });
  finish(emptyPage);
  await observation;
});

it('does not let an earlier session snapshot roll back an accepted rename', async () => {
  let emit!: (event: StreamEvent) => Promise<void>;
  vi.spyOn(DaemonClient.prototype, 'stream').mockImplementation(async (_path, onEvent, signal) => {
    emit = onEvent;
    await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  });
  let sessionReads = 0;
  let stale!: (value: unknown) => void;
  vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/session') {
      if (++sessionReads === 2) return await new Promise<T>((resolve) => { stale = resolve as (value: unknown) => void; });
      return { ...fixture.value, daemon_name: sessionReads === 1 ? 'Original' : 'Accepted name', daemon_name_generation: sessionReads === 1 ? 1 : 2, can_rename_daemon: true } as T;
    }
    if (path === '/v1/daemon/rename') return { request_id: (body as { request_id: string }).request_id, daemon_id: fixture.value.daemon_id, name: 'Accepted name', generation: 2 } as T;
    return { items: [], next_cursor: null, truncated: false } as T;
  });
  render(App);
  await fireEvent.click(screen.getByRole('button', { name: 'Connect a daemon' }));
  await authenticate();
  const observation = emit({ event: 'workflow_changed', data: '{}' });
  await fireEvent.click(screen.getByRole('button', { name: 'Daemon settings' }));
  await fireEvent.input(screen.getByLabelText('Daemon name'), { target: { value: 'Accepted name' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Save daemon name' }).closest('form')!);
  await screen.findByRole('button', { name: /^Accepted name/ });
  stale({ ...fixture.value, daemon_name: 'Original', daemon_name_generation: 1 });
  await observation;
  expect(screen.queryByRole('button', { name: /^Original/ })).toBeNull();
  expect((screen.getByLabelText('Daemon name') as HTMLInputElement).value).toBe('Accepted name');
});

it('retains an uncertain rename across navigation and replays its original request', async () => {
  idleStream();
  let renameCount = 0;
  let accepted: unknown;
  const request = vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/session') return { ...fixture.value, daemon_name: renameCount ? 'Recovered name' : 'Original', daemon_name_generation: renameCount ? 2 : 1, can_rename_daemon: true } as T;
    if (path === '/v1/daemon/rename') {
      renameCount++;
      if (renameCount === 1) { accepted = body; throw new Error('Reply lost'); }
      return { request_id: (body as { request_id: string }).request_id, daemon_id: fixture.value.daemon_id, name: 'Recovered name', generation: 2 } as T;
    }
    return { items: [], next_cursor: null, truncated: false } as T;
  });
  render(App);
  await fireEvent.click(screen.getByRole('button', { name: 'Connect a daemon' }));
  await authenticate();
  await fireEvent.click(screen.getByRole('button', { name: 'Daemon settings' }));
  await fireEvent.input(screen.getByLabelText('Daemon name'), { target: { value: 'Recovered name' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Save daemon name' }).closest('form')!);
  await fireEvent.click(screen.getByRole('button', { name: 'Back to work' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Daemon settings' }));
  expect((screen.getByLabelText('Daemon name') as HTMLInputElement).value).toBe('Recovered name');
  expect((screen.getByRole('button', { name: 'Save daemon name' }) as HTMLButtonElement).disabled).toBe(true);
  await fireEvent.click(screen.getByRole('button', { name: 'Replay exact rename' }));
  await screen.findByRole('button', { name: /^Recovered name/ });
  expect(request.mock.calls.filter(([path]) => path === '/v1/daemon/rename').map(([, body]) => body)).toEqual([accepted, accepted]);
});

it('clears an active draft when the connected endpoint reports a different daemon', async () => {
  let emit!: (event: StreamEvent) => Promise<void>;
  vi.spyOn(DaemonClient.prototype, 'stream').mockImplementation(async (_path, onEvent, signal) => {
    emit = onEvent;
    await new Promise<void>((resolve) => signal.addEventListener('abort', () => resolve(), { once: true }));
  });
  let replaced = false;
  vi.spyOn(DaemonClient.prototype, 'request').mockImplementation(async <T>(path: string): Promise<T> => {
    if (path === '/v1/session') return { ...fixture.value, daemon_name: replaced ? 'Replacement daemon' : 'Original daemon', daemon_id: replaced ? 'replacement-pin' : fixture.value.daemon_id } as T;
    return { items: [], next_cursor: null, truncated: false } as T;
  });
  render(App);
  await fireEvent.click(screen.getByRole('button', { name: 'Connect a daemon' }));
  await authenticate();
  await fireEvent.click(screen.getByRole('button', { name: '+ New task' }));
  await fireEvent.input(screen.getByLabelText('Task title'), { target: { value: 'Original daemon draft' } });
  replaced = true;
  await emit({ event: 'workflow_changed', data: '{}' });
  await waitFor(() => expect(screen.queryByLabelText('Task title')).toBeNull());
  expect(screen.getByRole('alert').textContent).toContain('different daemon');
  expect(screen.queryByRole('button', { name: 'Daemon settings' })).toBeNull();
  expect(screen.queryByRole('button', { name: /^Replacement daemon/ })).toBeNull();
  expect(JSON.parse(window.localStorage.getItem('milkdrift.connections.v1')!)[0].daemonId).toBe(fixture.value.daemon_id);
});
