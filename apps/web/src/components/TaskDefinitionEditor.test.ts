import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it } from 'vitest';
import TaskDefinitionEditorTestHarness from './TaskDefinitionEditorTestHarness.svelte';
import type { TaskDefinitionDto } from '../lib/protocol.generated';

afterEach(cleanup);

function definition(): TaskDefinitionDto {
  return { title: 'Research', description: '', instructions: 'Inspect the supplied context.', capability: { id: 'model', operation: 'model.generate' }, inputs: {}, outputs: {}, dependencies: [], required: true, corrective_of: null, paths: [{ path: '/project', write: false }], actions: { automatic_repair: true }, tools: [{ kind: 'read_file' }] };
}

function edited(): TaskDefinitionDto {
  return JSON.parse(screen.getByTestId('edited-definition').textContent!) as TaskDefinitionDto;
}

it('preserves model choices between models and removes them before selecting a process', async () => {
  const task = definition();
  render(TaskDefinitionEditorTestHarness, { initial: task, capabilities: [task.capability, { id: 'another-model', operation: 'model.generate' }, { id: 'process', operation: 'process.execute' }] });
  await fireEvent.change(screen.getByLabelText('Configured capability'), { target: { value: 'another-model/model.generate' } });
  expect((screen.getByLabelText('Allow automatic feedback after refused proposals') as HTMLInputElement).checked).toBe(true);
  expect((screen.getByLabelText('Permit bounded file reads in declared paths') as HTMLInputElement).checked).toBe(true);
  await fireEvent.change(screen.getByLabelText('Configured capability'), { target: { value: 'process/process.execute' } });
  expect(screen.queryByLabelText('Allow model to propose future work')).toBeNull();
  expect(edited()).toMatchObject({ capability: { id: 'process', operation: 'process.execute' }, actions: null, tools: [] });
  await fireEvent.change(screen.getByLabelText('Configured capability'), { target: { value: 'model/model.generate' } });
  expect((screen.getByLabelText('Allow model to propose future work') as HTMLInputElement).checked).toBe(false);
  expect((screen.getByLabelText('Permit bounded file reads in declared paths') as HTMLInputElement).checked).toBe(false);
});

it('also clears model choices when a manually entered operation changes', async () => {
  const task = definition();
  render(TaskDefinitionEditorTestHarness, { initial: task });
  await fireEvent.input(screen.getByLabelText('Operation'), { target: { value: 'process.execute' } });
  expect(screen.queryByLabelText('Allow model to propose future work')).toBeNull();
  expect(edited()).toMatchObject({ capability: { operation: 'process.execute' }, actions: null, tools: [] });
});

it('keeps the corrective link open and focused when its final character is cleared', async () => {
  const task = { ...definition(), corrective_of: 'original-task' };
  render(TaskDefinitionEditorTestHarness, { initial: task });
  const input = screen.getByLabelText('Corrects task') as HTMLInputElement;
  input.focus();
  await fireEvent.input(input, { target: { value: '' } });
  expect((input.closest('details') as HTMLDetailsElement).open).toBe(true);
  expect(document.activeElement).toBe(input);
  expect(edited().corrective_of).toBeNull();
  await fireEvent.input(input, { target: { value: 'another-task' } });
  expect(edited().corrective_of).toBe('another-task');
});
