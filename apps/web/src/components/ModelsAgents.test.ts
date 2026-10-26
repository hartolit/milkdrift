import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import ModelsAgents from './ModelsAgents.svelte';
import fixture from '../../../../crates/control-protocol/tests/fixtures/task-session.json';
import { DaemonClient } from '../lib/transport';
import type { TaskSessionDto } from '../lib/protocol.generated';

afterEach(cleanup);

function mount(session: TaskSessionDto = fixture.value) {
  const client = new DaemonClient('http://127.0.0.1:10001', 'test-credential');
  const onrefresh = vi.fn(async () => {});
  const onerror = vi.fn();
  const onback = vi.fn();
  return { client, onrefresh, onerror, onback, view: render(ModelsAgents, { session, client, onrefresh, onerror, onback }) };
}

it('shows the configured model label and explains where task assignment lives', () => {
  const capability = { id: 'configured-model', operation: 'model.generate' };
  mount({ ...fixture.value, capabilities: [capability], authoring: [{ capability, label: 'Local research model', generation: 3, category: 'model', input_media: 'text_json_or_image', inputs: [], outputs: [], default_output: null, allows_additional_inputs: true }] });
  expect(screen.getByRole('heading', { name: 'Local research model' })).toBeTruthy();
  expect(screen.getByText('Model or agent')).toBeTruthy();
  expect(screen.getByRole('region', { name: 'Models' }).textContent).toContain('Available to select');
});

it('makes setup truthful without submitting configuration or losing the return action', async () => {
  const { client, onback } = mount({ ...fixture.value, capabilities: [], authoring: [] });
  const request = vi.spyOn(client, 'request');
  await fireEvent.click(screen.getByRole('button', { name: 'Add model or agent' }));
  expect(screen.getByText(/installation is an operator step/)).toBeTruthy();
  expect(screen.getByRole('link', { name: /Open model setup guide/ }).getAttribute('href')).toContain('/docs/guides/local-model-endpoint.md');
  await fireEvent.click(screen.getByRole('button', { name: 'Command-line agent or tool' }));
  expect(screen.getByRole('link', { name: /Open process setup guide/ }).getAttribute('href')).toContain('/docs/guides/local-process.md');
  expect(request).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Back to canvas' }));
  expect(onback).toHaveBeenCalledOnce();
});

it('refreshes through the authenticated connection and reports a refusal to its owner', async () => {
  const { onrefresh, onerror } = mount();
  const failure = new Error('Connection is no longer authorized');
  onrefresh.mockRejectedValueOnce(failure);
  await fireEvent.click(screen.getByRole('button', { name: 'Refresh choices' }));
  expect(onerror).toHaveBeenCalledWith(failure);
});
