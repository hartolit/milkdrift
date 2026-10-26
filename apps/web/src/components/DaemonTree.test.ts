import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import DaemonTree from './DaemonTree.svelte';
import { project, session, workflow } from '../lib/canvas.test-fixtures';
import { DaemonClient } from '../lib/transport';

beforeEach(() => { localStorage.clear(); localStorage.setItem('milkdrift.tree.v1', JSON.stringify(['daemon-a/actor-a/', `daemon-a/actor-a/${project.id}`])); });
afterEach(() => { cleanup(); localStorage.clear(); vi.restoreAllMocks(); });
function setup(client = new DaemonClient('http://127.0.0.1:3000', 'private-token')) {
  const profile = { id: 'profile-a', cachedName: 'Studio A', endpoint: client.endpoint, daemonId: 'daemon-a' };
  const selected = workflow(), branch = { profile, client, session: session(), projects: [project], projectCursor: null, selectedId: selected.id, selected };
  const props = { profiles: [profile], connections: [branch], activeId: profile.id, onchoose: vi.fn(), onselect: vi.fn(), onnew: vi.fn(), onproject: vi.fn(), onmoreprojects: vi.fn(), onsettings: vi.fn(), onmodels: vi.fn(), onerror: vi.fn() };
  return { client, branch, props };
}
it('refreshes the selected workflow state without refreshing for progress generations alone', async () => {
  const { client, branch, props } = setup(); let observed = workflow();
  const request = vi.spyOn(client, 'request').mockImplementation(async <T>(): Promise<T> => ({ items: [observed], next_cursor: null, truncated: false }) as T);
  const view = render(DaemonTree, props); await screen.findByRole('button', { name: /Research workflow.*waiting/ }); const reads = request.mock.calls.length;
  const progress = { ...branch.selected, generation: 99, updated_at: 50 };
  await view.rerender({ ...props, connections: [{ ...branch, selected: progress }] });
  expect(request).toHaveBeenCalledTimes(reads);
  observed = { ...observed, state: 'completed' };
  const settled = { ...observed, generation: 100 };
  await view.rerender({ ...props, connections: [{ ...branch, selected: settled }] });
  await screen.findByRole('button', { name: /Research workflow.*completed/ }); expect(request).toHaveBeenCalledTimes(reads + 1);
});
it('drops old tree data and ignores a late failure when the authenticated client is replaced', async () => {
  const { client, branch, props } = setup();
  const request = vi.spyOn(client, 'request').mockImplementation(async <T>(): Promise<T> => ({ items: [workflow()], next_cursor: null, truncated: false }) as T);
  const view = render(DaemonTree, props); await screen.findByRole('button', { name: /Research workflow/ });
  await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'Private old query' } });
  let reject!: (failure: Error) => void;
  request.mockImplementation(() => new Promise((_, failure) => { reject = failure; }));
  await fireEvent.click(screen.getByRole('button', { name: 'Refresh workflows' }));
  const next = new DaemonClient(client.endpoint, 'replacement-token');
  vi.spyOn(next, 'request').mockImplementation(async <T>(): Promise<T> => ({ items: [], next_cursor: null, truncated: false }) as T);
  await view.rerender({ ...props, connections: [{ ...branch, client: next, selectedId: '', selected: null }] });
  await waitFor(() => expect(screen.queryByRole('button', { name: /Research workflow/ })).toBeNull());
  expect((screen.getByRole('searchbox') as HTMLInputElement).value).toBe('');
  reject(new Error('Old client failed after replacement')); await Promise.resolve(); expect(props.onerror).not.toHaveBeenCalled();
});
