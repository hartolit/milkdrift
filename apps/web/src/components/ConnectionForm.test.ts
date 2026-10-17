import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import ConnectionForm from './ConnectionForm.svelte';

afterEach(cleanup);

it('clears the credential field when connecting and exposes a failed connection accessibly', async () => {
  const onconnect = vi.fn(async () => { throw new Error('This address now connects to a different daemon.'); });
  render(ConnectionForm, { onconnect, oncancel: vi.fn() });
  expect(screen.queryByLabelText('Connection name')).toBeNull();
  expect(screen.queryByLabelText('Installation identity')).toBeNull();
  await fireEvent.input(screen.getByLabelText('Daemon URL'), { target: { value: 'https://studio.example' } });
  await fireEvent.input(screen.getByLabelText('Bearer credential'), { target: { value: 'private-token' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Connect' }).closest('form')!);
  expect(onconnect).toHaveBeenCalledWith({ endpoint: 'https://studio.example', token: 'private-token' });
  expect((screen.getByLabelText('Bearer credential') as HTMLInputElement).value).toBe('');
  expect(screen.getByRole('alert').textContent).toContain('This address now connects to a different daemon.');
});
