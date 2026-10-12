import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import TaskComposer from './TaskComposer.svelte';
import fixture from '../../../../crates/control-protocol/tests/fixtures/task-session.json';
import { DaemonClient } from '../lib/transport';

afterEach(cleanup);

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
