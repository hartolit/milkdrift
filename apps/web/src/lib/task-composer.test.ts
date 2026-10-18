import { expect, it } from 'vitest';
import fixture from '../../../../crates/control-protocol/tests/fixtures/task-session.json';
import { capabilityKey, createTaskDraft, deriveTaskTitle, freezeTaskSubmission, updateDraftContext, updateDraftInstructions } from './task-composer';

const session = { ...fixture.value, capabilities: [{ id: 'process', operation: 'process.execute' }] };
it('bounds derived Unicode titles by UTF-8 bytes and preserves code points', () => {
  const title = deriveTaskTitle(`  ${'😀'.repeat(40)}\nIgnored second line`);
  expect(new TextEncoder().encode(title).length).toBe(96);
  expect(title).toBe('😀'.repeat(24));
  expect(deriveTaskTitle('  Compare   libraries\nDetails')).toBe('Compare libraries');
  const draft = createTaskDraft(session, { kind: 'new' });
  updateDraftInstructions(draft, 'Valid instructions');
  draft.definition.title = '😀'.repeat(65);
  expect(() => freezeTaskSubmission(draft, session)).toThrow('256 UTF-8 bytes');
});

it('validates deliberate previous executor choice against the current session', () => {
  const two = { ...session, capabilities: [...session.capabilities, { id: 'model', operation: 'model.generate' }] };
  expect(createTaskDraft(two, { kind: 'new' }).definition.capability.id).toBe('');
  expect(createTaskDraft(two, { kind: 'new' }, two.capabilities[1]).definition.capability).toEqual(two.capabilities[1]);
  expect(createTaskDraft(two, { kind: 'new' }, { id: 'gone', operation: 'process.execute' }).definition.capability.id).toBe('');
  expect(capabilityKey({ id: 'a/b', operation: 'c' })).not.toBe(capabilityKey({ id: 'a', operation: 'b/c' }));
});

it('freezes one add without resume, limit changes or shared input replacement', () => {
  const draft = createTaskDraft(session, { kind: 'existing', workflowId: 'A', title: 'A', generation: 3 });
  updateDraftInstructions(draft, 'An additional task');
  draft.contextText = 'A private context'; updateDraftContext(draft);
  const frozen = freezeTaskSubmission(draft, session);
  draft.definition.instructions = 'Later edit';
  draft.definition.inputs = {};
  expect(frozen).toMatchObject({ path: '/v1/workflows/A/changes', request: { expected_generation: 3, changes: [{ kind: 'add', definition: { instructions: 'An additional task', inputs: { brief: { kind: 'inline', value: { kind: 'text', text: 'A private context' } } } } }] } });
  expect('changes' in frozen.request && frozen.request.changes).toHaveLength(1);
});

it('allocates separate repair inputs and refuses submitting a draft under another identity', () => {
  const first = createTaskDraft(session, { kind: 'new' }), second = createTaskDraft(session, { kind: 'new' });
  for (const draft of [first, second]) { updateDraftInstructions(draft, 'Needs later input'); draft.supplyContext = false; updateDraftContext(draft); }
  expect(first.definition.inputs.brief).not.toEqual(second.definition.inputs.brief);
  expect(() => freezeTaskSubmission(first, { ...session, daemon_id: 'another' })).toThrow('different authenticated connection');
  first.disposed = true;
  expect(() => freezeTaskSubmission(first, session)).toThrow('different authenticated connection');
});
