import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import DaemonSettings from './DaemonSettingsTestHarness.svelte';
import { DaemonClient, DaemonError } from '../lib/transport';
import fixture from '../../../../crates/control-protocol/tests/fixtures/task-session.json';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

function settings(canRename = true) {
  const client = new DaemonClient('http://127.0.0.1:10001', 'unit-test-credential');
  const session = { ...fixture.value, daemon_name: 'Studio A', daemon_name_generation: 3, can_rename_daemon: canRename };
  const onrenamed = vi.fn();
  const onrefresh = vi.fn(async () => {});
  const onerror = vi.fn();
  const result = render(DaemonSettings, { client, session, onrenamed, onrefresh, onerror, onback: vi.fn() });
  return { ...result, client, session, onrenamed, onrefresh, onerror };
}

it('replays the identical rename after a lost reply and does not create another request', async () => {
  const { client, session, onrenamed } = settings();
  const request = vi.spyOn(client, 'request').mockRejectedValueOnce(new Error('Reply lost'));
  await fireEvent.input(screen.getByLabelText('Daemon name'), { target: { value: ' Animation studio ' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Save daemon name' }).closest('form')!);
  expect(screen.getByText(/The rename may have been accepted/)).toBeTruthy();
  expect((screen.getByLabelText('Daemon name') as HTMLInputElement).readOnly).toBe(true);
  const accepted = request.mock.calls[0]![1] as { request_id: string };
  request.mockResolvedValueOnce({ request_id: accepted.request_id, daemon_id: session.daemon_id, name: 'Animation studio', generation: 4 });
  await fireEvent.click(screen.getByRole('button', { name: 'Replay exact rename' }));
  await waitFor(() => expect(onrenamed).toHaveBeenCalledTimes(1));
  expect(request.mock.calls[1]).toEqual(request.mock.calls[0]);
  expect(request.mock.calls[0]?.[1]).toMatchObject({ expected_generation: 3, name: ' Animation studio ' });
});

it('finds a lost rename receipt without sending another mutation', async () => {
  const { client, session, onrenamed } = settings();
  const request = vi.spyOn(client, 'request').mockRejectedValueOnce(new Error('Reply lost'));
  await fireEvent.input(screen.getByLabelText('Daemon name'), { target: { value: 'New studio' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Save daemon name' }).closest('form')!);
  const accepted = request.mock.calls[0]![1] as { request_id: string };
  request.mockResolvedValueOnce({ request_id: accepted.request_id, daemon_id: session.daemon_id, name: 'New studio', generation: 4 });
  await fireEvent.click(screen.getByRole('button', { name: 'Find rename receipt' }));
  await waitFor(() => expect(onrenamed).toHaveBeenCalledTimes(1));
  expect(request.mock.calls[1]?.slice(0, 2)).toEqual([`/v1/daemon/rename-requests/${accepted.request_id}`, undefined]);
});

it('retains a conflicting draft for an explicit new decision after refreshing current metadata', async () => {
  const { client, onrefresh, rerender, session } = settings();
  const request = vi.spyOn(client, 'request').mockRejectedValueOnce(new DaemonError('conflict', 'The daemon name changed.', 409));
  await fireEvent.input(screen.getByLabelText('Daemon name'), { target: { value: 'My draft' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Save daemon name' }).closest('form')!);
  expect(onrefresh).toHaveBeenCalledTimes(1);
  await rerender({ session: { ...session, daemon_name: 'Other client name', daemon_name_generation: 4 } });
  expect((screen.getByLabelText('Daemon name') as HTMLInputElement).value).toBe('My draft');
  expect(screen.queryByRole('button', { name: 'Replay exact rename' })).toBeNull();
  request.mockRejectedValueOnce(new DaemonError('conflict', 'Another conflict', 409));
  await fireEvent.submit(screen.getByRole('button', { name: 'Save daemon name' }).closest('form')!);
  expect(request.mock.calls[1]?.[1]).toMatchObject({ expected_generation: 4, name: 'My draft' });
  expect(request.mock.calls[1]?.[1]).not.toEqual(request.mock.calls[0]?.[1]);
});

it('shows a read-only name without exposing a rename submission', async () => {
  const { client } = settings(false);
  const request = vi.spyOn(client, 'request');
  expect((screen.getByLabelText('Daemon name') as HTMLInputElement).value).toBe('Studio A');
  expect((screen.getByLabelText('Daemon name') as HTMLInputElement).readOnly).toBe(true);
  expect(screen.queryByRole('button', { name: 'Save daemon name' })).toBeNull();
  await fireEvent.submit(screen.getByLabelText('Daemon name').closest('form')!);
  expect(request).not.toHaveBeenCalled();
});

it('ignores an accepted reply after the settings connection is destroyed', async () => {
  const { client, session, onrenamed, unmount } = settings();
  let resolve!: (receipt: unknown) => void;
  const request = vi.spyOn(client, 'request').mockImplementation(() => new Promise((done) => { resolve = done; }));
  await fireEvent.input(screen.getByLabelText('Daemon name'), { target: { value: 'Late name' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Save daemon name' }).closest('form')!);
  const accepted = request.mock.calls[0]![1] as { request_id: string };
  unmount();
  resolve({ request_id: accepted.request_id, daemon_id: session.daemon_id, name: 'Late name', generation: 4 });
  await Promise.resolve();
  expect(onrenamed).not.toHaveBeenCalled();
});
