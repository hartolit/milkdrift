import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import ConnectionForm from './ConnectionForm.svelte';

afterEach(cleanup);

it('clears the credential field when connecting and exposes a failed connection accessibly', async () => {
  const onconnect = vi.fn(async () => { throw new Error('Installation identity does not match.'); });
  render(ConnectionForm, { onconnect, oncancel: vi.fn() });
  await fireEvent.input(screen.getByLabelText('Connection name'), { target: { value: 'Studio' } });
  await fireEvent.input(screen.getByLabelText('Daemon URL'), { target: { value: 'https://studio.example' } });
  await fireEvent.input(screen.getByLabelText('Installation identity'), { target: { value: 'studio-a' } });
  await fireEvent.input(screen.getByLabelText('Bearer credential'), { target: { value: 'private-token' } });
  await fireEvent.submit(screen.getByRole('button', { name: 'Connect' }).closest('form')!);
  expect(onconnect).toHaveBeenCalledWith({ label: 'Studio', endpoint: 'https://studio.example', daemonId: 'studio-a', token: 'private-token' });
  expect((screen.getByLabelText('Bearer credential') as HTMLInputElement).value).toBe('');
  expect(screen.getByRole('alert').textContent).toContain('Installation identity does not match.');
});
