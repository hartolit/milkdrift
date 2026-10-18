import type { ChangeWorkflowRequest, CreateWorkflowRequest, TaskCapability, TaskDefinitionDto, TaskSessionDto, TaskUploadReceipt, TaskUploadRequest, WorkflowLimitsDto } from './protocol.generated';

export type TaskDraftTarget = { kind: 'new' } | { kind: 'existing'; workflowId: string; title: string; generation: number };

export interface DefinitionEditorDraft {
  inputName: string; inputKind: 'inline' | 'supplied' | 'task_output' | 'settled_outcome'; inputSource: string;
  inputOutput: string; inputOptional: boolean; inputAbsent: boolean; outputName: string; dependency: string; path: string; write: boolean; processSelection: string;
}

export function createDefinitionEditorDraft(): DefinitionEditorDraft {
  return { inputName: '', inputKind: 'supplied', inputSource: '', inputOutput: 'result', inputOptional: false, inputAbsent: false, outputName: '', dependency: '', path: '', write: false, processSelection: '' };
}

/** A memory-only draft belongs to one authenticated connection and one destination. */
export interface TaskDraft {
  id: string; daemonId: string; actorId: string; target: TaskDraftTarget; disposed: boolean;
  definition: TaskDefinitionDto; titleEdited: boolean; workflowTitle: string;
  contextOpen: boolean; contextText: string; supplyContext: boolean; advancedOpen: boolean;
  editor: DefinitionEditorDraft; finishWhenSettled: boolean; limits: WorkflowLimitsDto | null;
  pending: { path: string; request: CreateWorkflowRequest | ChangeWorkflowRequest } | null;
  pendingUpload: TaskUploadRequest | null; upload: TaskUploadReceipt | null;
  busy: boolean; error: string; conflict: boolean; accepted: boolean;
}

export const capabilityKey = (capability: TaskCapability): string => JSON.stringify([capability.id, capability.operation]);

export function createTaskDraft(session: TaskSessionDto, target: TaskDraftTarget, preferredCapability?: TaskCapability): TaskDraft {
  const preferred = preferredCapability && session.capabilities.find((item) => capabilityKey(item) === capabilityKey(preferredCapability));
  const capability = preferred ?? (session.capabilities.length === 1 ? session.capabilities[0] : undefined);
  return {
    id: crypto.randomUUID(), daemonId: session.daemon_id, actorId: session.actor_id, target: { ...target }, disposed: false,
    definition: { title: '', description: '', instructions: '', capability: { ...(capability ?? { id: '', operation: '' }) }, inputs: {}, outputs: {}, dependencies: [], required: true, corrective_of: null, paths: [], actions: null, tools: [] },
    titleEdited: false, workflowTitle: '', contextOpen: false, contextText: '', supplyContext: true, advancedOpen: false,
    editor: createDefinitionEditorDraft(), finishWhenSettled: true, limits: null,
    pending: null, pendingUpload: null, upload: null, busy: false, error: '', conflict: false, accepted: false,
  };
}

/** Derive a short first-line title without splitting a Unicode code point. */
export function deriveTaskTitle(instructions: string): string {
  const line = instructions.trim().split(/\r?\n/, 1)[0]?.replace(/\s+/g, ' ') ?? '';
  let title = '';
  for (const character of line) {
    if (new TextEncoder().encode(title + character).length > 96) break;
    title += character;
  }
  return title.trim();
}

export function updateDraftInstructions(draft: TaskDraft, instructions: string): void {
  draft.definition.instructions = instructions;
  if (!draft.titleEdited) draft.definition.title = deriveTaskTitle(instructions);
}

/** Compact context is local to this task; deferring it allocates a distinct repair key. */
export function updateDraftContext(draft: TaskDraft): void {
  draft.definition.inputs = { ...draft.definition.inputs, brief: draft.supplyContext
    ? { kind: 'inline', value: { kind: 'text', text: draft.contextText } }
    : { kind: 'supplied', name: `context_${draft.id.replaceAll('-', '')}`, optional: false } };
}

export function freezeTaskSubmission(draft: TaskDraft, session: TaskSessionDto): NonNullable<TaskDraft['pending']> {
  if (draft.disposed || draft.daemonId !== session.daemon_id || draft.actorId !== session.actor_id) throw new Error('This draft belongs to a different authenticated connection.');
  if (!session.capabilities.some((item) => capabilityKey(item) === capabilityKey(draft.definition.capability))) throw new Error('Choose an executor currently permitted on this daemon. Your draft is retained.');
  const bytes = (value: string) => new TextEncoder().encode(value).length;
  if (!draft.definition.instructions.trim()) throw new Error('Describe what this task should do.');
  if (bytes(draft.definition.instructions) > session.input_limit_bytes) throw new Error(`Instructions exceed the daemon text bound of ${session.input_limit_bytes.toLocaleString()} UTF-8 bytes.`);
  for (const binding of Object.values(draft.definition.inputs)) {
    if (binding.kind === 'inline' && binding.value.kind === 'text' && bytes(binding.value.text) > session.input_limit_bytes) throw new Error(`A task input exceeds the daemon text bound of ${session.input_limit_bytes.toLocaleString()} UTF-8 bytes.`);
  }
  if (!draft.definition.title.trim() || bytes(draft.definition.title) > 256) throw new Error('Use a task title of 1–256 UTF-8 bytes.');
  if (bytes(draft.definition.description) > 4096) throw new Error('The short description exceeds 4,096 UTF-8 bytes.');
  const definition: TaskDefinitionDto = JSON.parse(JSON.stringify(draft.definition));
  const request_id = crypto.randomUUID();
  if (draft.target.kind === 'existing') return {
    path: `/v1/workflows/${encodeURIComponent(draft.target.workflowId)}/changes`,
    request: { request_id, expected_generation: draft.target.generation, changes: [{ kind: 'add', alias: 'task', definition }] },
  };
  const title = draft.workflowTitle.trim() || definition.title;
  if (bytes(title) > 256) throw new Error('Use a workflow name of at most 256 UTF-8 bytes.');
  return { path: '/v1/workflows', request: { request_id, title, description: definition.description, inputs: {}, tasks: [{ alias: 'task', definition }], finish_when_settled: draft.finishWhenSettled, limits: draft.limits ? { ...draft.limits } : null } };
}
