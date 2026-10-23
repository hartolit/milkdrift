import { readableReason, stateLabel } from './presentation';
import type { ChangeWorkflowRequest, CreateWorkflowRequest, InputBinding, InputValue, ProjectDto, TaskCapability, TaskChangeDto, TaskDefinitionDto, TaskReadDto, TaskSessionDto, TaskUploadRequest, WorkflowLimitsDto, WorkflowReadDto } from './protocol.generated';

export const canvasBounds = { changes: 32, tasks: 200, sources: 400, edges: 1024, draftBytes: 2 * 1024 * 1024, cacheBytes: 8 * 1024 * 1024 } as const;
export const capabilityKey = (capability: TaskCapability): string => JSON.stringify([capability.id, capability.operation]);
export const copy = <T>(value: T): T => JSON.parse(JSON.stringify(value));
export const byteLength = (value: string): number => new TextEncoder().encode(value).length;
export type CanvasTarget = { kind: 'new'; projectId?: string } | { kind: 'existing'; workflowId: string; title: string; generation: number; projectId?: string | null };
export interface Point { x: number; y: number }
export function positionCapacity(canvas: CanvasState): void { if (Object.keys(canvas.positions).length >= 600) throw new Error('The canvas has reached its 600-card layout bound. Clear cached details from Workflow to free old layout entries; drafts remain intact.'); }
function freePosition(canvas: CanvasState, source: boolean): Point {
  const occupied = Object.values(canvas.positions);
  for (let index = 0; index <= occupied.length; index++) {
    const candidate = { x: source ? 0 : 310 + index % 2 * 310, y: (source ? index : Math.floor(index / 2)) * 195 + 80 };
    if (!occupied.some((point) => Math.abs(point.x - candidate.x) < 270 && Math.abs(point.y - candidate.y) < 175)) return candidate;
  }
  return { x: source ? 0 : 310, y: (occupied.length + 1) * 195 + 80 };
}
export interface CanvasTask { id: string; definition: TaskDefinitionDto; version: number | null; titleEdited: boolean; contractGeneration: number | null; explicitDependencies: string[] }
export interface CanvasSource { id: string; label: string; value: InputValue | null; sharedName: string | null; text: string; deferred: boolean }
export interface CanvasState {
  id: string; daemonId: string; actorId: string; target: CanvasTarget; project: ProjectDto | null; disposed: boolean; capacityBlocked: boolean;
  title: string; tasks: CanvasTask[]; sources: CanvasSource[]; changes: TaskChangeDto[];
  details: TaskReadDto[]; positions: Record<string, Point>; bindings: Record<string, string>; camera: Point & { zoom: number };
  selected: string; inspector: boolean; presentation: 'canvas' | 'list' | 'timeline';
  finishWhenSettled: boolean; limits: WorkflowLimitsDto | null;
  pending: { path: string; request: CreateWorkflowRequest | ChangeWorkflowRequest; control?: boolean } | null;
  pendingUpload: { request: TaskUploadRequest; target: string } | null;
  busy: boolean; error: string; conflict: boolean; notice: string;
}

export interface GraphNode { id: string; kind: 'task' | 'source' | 'boundary'; label: string; detail: string; state: string; status?: string; draft: boolean; position: Point }
export interface GraphEdge { id: string; source: string; target: string; label: string; input: string | null; binding: InputBinding | null; draft: boolean }

export function createCanvas(session: TaskSessionDto, target: CanvasTarget, project: ProjectDto | null = null): CanvasState {
  return { id: crypto.randomUUID(), daemonId: session.daemon_id, actorId: session.actor_id, target: { ...target }, project: project ? copy(project) : null, disposed: false, capacityBlocked: false, title: '', tasks: [], sources: [], changes: [], details: [], positions: {}, bindings: {}, camera: { x: 50, y: 60, zoom: 1 }, selected: '', inspector: false, presentation: 'canvas', finishWhenSettled: true, limits: null, pending: null, pendingUpload: null, busy: false, error: '', conflict: false, notice: '' };
}

export function hasChanges(canvas: CanvasState): boolean { return !!(canvas.tasks.length || canvas.sources.length || canvas.changes.length || canvas.pending || canvas.pendingUpload); }
export function changeCount(canvas: CanvasState): number { return canvas.tasks.length + canvas.changes.length; }

export function deriveTaskTitle(instructions: string): string {
  const first = instructions.trim().split(/\r?\n/, 1)[0]?.replace(/\s+/g, ' ') ?? '';
  let title = '';
  for (const character of first) { if (byteLength(title + character) > 96) break; title += character; }
  return title.trim();
}

export function assertEditable(canvas: CanvasState): void {
  if (canvas.disposed) throw new Error('This authenticated workspace is no longer available.');
  if (canvas.capacityBlocked) throw new Error('This connection already holds 8 drafts. Open and discard one before changing another workflow. You can keep browsing; your drawings are intact.');
  if (canvas.pending || canvas.pendingUpload || canvas.busy) throw new Error('Recover the pending request before changing this drawing.');
}

export function assertCapacity(canvas: CanvasState, extraChanges = 0): void {
  if (changeCount(canvas) + extraChanges > canvasBounds.changes) throw new Error(`Apply or discard some changes before adding more. One submission can contain at most ${canvasBounds.changes} changes; the drawing is intact.`);
  if (byteLength(JSON.stringify({ tasks: canvas.tasks, sources: canvas.sources, changes: canvas.changes })) > canvasBounds.draftBytes) throw new Error('This drawing has reached its 2 MiB draft limit. Shorten or discard an input before adding more.');
}

export function addCanvasTask(canvas: CanvasState, session: TaskSessionDto, preferred?: TaskCapability): CanvasTask {
  assertEditable(canvas); assertCapacity(canvas, 1);
  positionCapacity(canvas);
  const available = session.capabilities;
  const capability = available.find((item) => preferred && capabilityKey(item) === capabilityKey(preferred)) ?? (available.length === 1 ? available[0] : undefined);
  const id = `draft_${crypto.randomUUID().replaceAll('-', '')}`;
  const task: CanvasTask = { id, version: null, titleEdited: false, contractGeneration: capability ? authoring(session, capability)?.generation ?? null : null, explicitDependencies: [], definition: { title: '', description: '', instructions: '', capability: copy(capability ?? { id: '', operation: '' }), inputs: {}, outputs: {}, dependencies: [], required: true, corrective_of: null, paths: [], actions: null, tools: [], execution: canvas.project ? { kind: 'project', expected_generation: canvas.project.generation } : { kind: 'legacy' } } };
  const before = canvas.tasks;
  canvas.tasks = [...canvas.tasks, task];
  try { assertCapacity(canvas); } catch (failure) { canvas.tasks = before; throw failure; }
  canvas.selected = id; canvas.inspector = true;
  canvas.positions[id] = freePosition(canvas, false);
  return canvas.tasks.find((item) => item.id === id)!;
}

export function reviseTask(canvas: CanvasState, task: TaskReadDto, session: TaskSessionDto): CanvasTask {
  const existing = canvas.tasks.find((item) => item.id === task.id);
  if (existing) return existing;
  assertEditable(canvas); assertCapacity(canvas, 1);
  if (task.state !== 'queued' || task.started_at !== null) throw new Error('This task has already started. Create a correction to change future work.');
  // Accepted records do not distinguish a wire-implied prerequisite from an explicit one.
  // Preserve every accepted prerequisite; removing its separate Run after edge is deliberate.
  const draft: CanvasTask = { id: task.id, version: task.version, titleEdited: true, contractGeneration: authoring(session, task.definition.capability)?.generation ?? null, explicitDependencies: [...task.definition.dependencies], definition: copy(task.definition) };
  const before = canvas.tasks;
  canvas.tasks = [...canvas.tasks, draft];
  try { assertCapacity(canvas); } catch (failure) { canvas.tasks = before; throw failure; }
  return canvas.tasks.find((item) => item.id === draft.id)!;
}

export function authoring(session: TaskSessionDto, capability: TaskCapability) {
  return session.authoring?.find((item) => capabilityKey(item.capability) === capabilityKey(capability));
}

export function chooseCapability(task: CanvasTask, session: TaskSessionDto, key: string): void {
  const capability = session.capabilities.find((item) => capabilityKey(item) === key);
  if (!capability) throw new Error('That model or agent is no longer available. Choose a current connection.');
  task.definition.capability = copy(capability);
  task.contractGeneration = authoring(session, capability)?.generation ?? null;
  if (authoring(session, capability)?.category !== 'model') { task.definition.tools = []; task.definition.paths = []; task.definition.actions = null; }
}

export function updateDefinition(canvas: CanvasState, task: CanvasTask, definition: TaskDefinitionDto): void {
  assertEditable(canvas);
  const previous = task.definition;
  task.definition = definition;
  try { assertCapacity(canvas); } catch (failure) { task.definition = previous; throw failure; }
}

export function updateInstructions(canvas: CanvasState, task: CanvasTask, text: string): void {
  assertEditable(canvas);
  const previous = task.definition.instructions, title = task.definition.title;
  task.definition.instructions = text;
  if (!task.titleEdited) task.definition.title = deriveTaskTitle(text);
  try { assertCapacity(canvas); } catch (failure) { task.definition.instructions = previous; task.definition.title = title; throw failure; }
}

export function addSource(canvas: CanvasState, label = 'Text input', value: InputValue | null = { kind: 'text', text: '' }): CanvasSource {
  assertEditable(canvas); assertCapacity(canvas);
  positionCapacity(canvas);
  if (canvas.sources.length >= canvasBounds.sources) throw new Error('This canvas has reached its 400-source limit. Discard an unused source before adding another.');
  const id = `source_${crypto.randomUUID().replaceAll('-', '')}`;
  const source = { id, label, value, sharedName: null, text: value?.kind === 'text' ? value.text : '', deferred: value === null };
  const before = canvas.sources;
  canvas.sources = [...canvas.sources, source];
  try { assertCapacity(canvas); } catch (failure) { canvas.sources = before; throw failure; }
  canvas.positions[id] = freePosition(canvas, true); canvas.selected = id; canvas.inspector = true;
  return canvas.sources.find((item) => item.id === id)!;
}

/** The source owns its binding value; generated keys have no authoring UI. */
export function sourceBinding(source: CanvasSource): InputBinding {
  return source.deferred || source.sharedName ? { kind: 'supplied', name: source.sharedName ?? source.id, optional: false } : { kind: 'inline', value: copy(source.value ?? { kind: 'text', text: source.text }) };
}

export function updateSource(canvas: CanvasState, source: CanvasSource, text: string, limit: number, deferred = source.deferred): void {
  assertEditable(canvas);
  if (byteLength(text) > limit) throw new Error(`This input exceeds the ${limit.toLocaleString()} byte limit. Its previous content is intact.`);
  const previous = copy(source);
  const inputs = canvas.tasks.map((task) => ({ task, inputs: copy(task.definition.inputs) }));
  source.text = text; source.value = { kind: 'text', text }; source.deferred = deferred;
  source.label = deriveTaskTitle(text).slice(0, 48) || 'Text input';
  for (const task of canvas.tasks) {
    for (const [name] of Object.entries(task.definition.inputs)) if (canvas.bindings[`${task.id}:${name}`] === source.id) task.definition.inputs[name] = sourceBinding(source);
  }
  try { assertCapacity(canvas); } catch (failure) { Object.assign(source, previous); for (const item of inputs) item.task.definition.inputs = item.inputs; throw failure; }
}

export function inputSlots(task: CanvasTask, session: TaskSessionDto) {
  const contract = authoring(session, task.definition.capability);
  if (!contract) throw new Error('Input choices are unavailable for this model or agent. Refresh Models & agents; this draft is retained.');
  const slots = contract.inputs.filter((input) => !(input.name in task.definition.inputs));
  return { slots, additional: contract.allows_additional_inputs };
}

export function connectBinding(canvas: CanvasState, task: CanvasTask, binding: InputBinding, session: TaskSessionDto, slot?: string, sourceId?: string): string {
  assertEditable(canvas);
  const { slots, additional } = inputSlots(task, session);
  const name = slot ?? (slots.length === 1 && !additional ? slots[0].name : additional ? sourceId ?? `input_${crypto.randomUUID().replaceAll('-', '')}` : '');
  if (!name || (!additional && !slots.some((entry) => entry.name === name))) throw new Error('Choose one of this agent’s available input connections.');
  if (Object.keys(task.definition.inputs).length >= 32) throw new Error('This task already has 32 input connections. Remove one before connecting another.');
  if (binding.kind === 'task_output' && slots.find((entry) => entry.name === name)?.accepts_artifacts === false) throw new Error('This agent input accepts text, not a result artifact. Connect text or choose an agent with a file input.');
  const previous = copy(task.definition);
  task.definition.inputs[name] = copy(binding);
  synchronizeDependencies(task);
  try { assertCapacity(canvas); } catch (failure) { task.definition = previous; throw failure; }
  if (sourceId) canvas.bindings[`${task.id}:${name}`] = sourceId;
  return name;
}

export function synchronizeDependencies(task: CanvasTask): void {
  task.definition.dependencies = [...new Set([...task.explicitDependencies, ...Object.values(task.definition.inputs).flatMap((input) => input.kind === 'task_output' || input.kind === 'settled_outcome' ? [input.task] : [])])];
}

export function removeConnection(task: CanvasTask, input: string): void { const inputs = { ...task.definition.inputs }; delete inputs[input]; task.definition.inputs = inputs; synchronizeDependencies(task); }

export function stageChange(canvas: CanvasState, change: TaskChangeDto): void {
  assertEditable(canvas); assertCapacity(canvas, 1);
  const before = canvas.changes; canvas.changes = [...canvas.changes, copy(change)];
  try { assertCapacity(canvas); } catch (failure) { canvas.changes = before; throw failure; }
}

export function freezeCanvas(canvas: CanvasState, session: TaskSessionDto): NonNullable<CanvasState['pending']> {
  if (canvas.disposed || canvas.daemonId !== session.daemon_id || canvas.actorId !== session.actor_id) throw new Error('This drawing belongs to another authenticated connection.');
  assertCapacity(canvas);
  if (!changeCount(canvas)) throw new Error('Add a task or make a change before submitting.');
  const unused = canvas.sources.find((source) => !canvas.tasks.some((task) => Object.keys(task.definition.inputs).some((name) => canvas.bindings[`${task.id}:${name}`] === source.id)));
  if (unused) throw new Error(`Connect or discard “${unused.label}” before applying this drawing. Unconnected input cards cannot be saved on their own.`);
  const tasks = canvas.tasks.map((task) => {
    const contract = authoring(session, task.definition.capability);
    if (!session.capabilities.some((item) => capabilityKey(item) === capabilityKey(task.definition.capability))) throw new Error(`Choose a model or agent for ${task.definition.title || 'the new task'}.`);
    if (!contract || task.contractGeneration !== null && task.contractGeneration !== contract.generation) throw new Error(`The model or agent for ${task.definition.title || 'this task'} changed. Reselect it and review its inputs before submitting.`);
    for (const input of contract.inputs) if (input.required && !(input.name in task.definition.inputs)) throw new Error(`${task.definition.title || 'This task'} needs ${input.label}. Use Add input or connect a source before starting.`);
    if (!contract.allows_additional_inputs && Object.keys(task.definition.inputs).some((name) => !contract.inputs.some((input) => input.name === name))) throw new Error(`Some connections do not match the selected agent for ${task.definition.title}. Remove and reconnect those inputs.`);
    for (const [name, output] of Object.entries(task.definition.outputs)) {
      const port = contract.outputs.find((item) => item.name === name);
      if (!port || port.media_type !== null && output.media_type !== null && port.media_type !== output.media_type) throw new Error(`A connected result is not published by the selected model or agent for ${task.definition.title}. Remove its outgoing connection and select a result from the current agent.`);
    }
    if (!task.definition.instructions.trim()) throw new Error('Describe what each new task should do.');
    if (byteLength(task.definition.instructions) > Math.min(16384, session.input_limit_bytes)) throw new Error('Task instructions exceed the 16 KiB limit. Shorten them or attach context separately.');
    if (!task.definition.title.trim() || byteLength(task.definition.title) > 256) throw new Error('Use task titles of 1–256 UTF-8 bytes.');
    for (const input of Object.values(task.definition.inputs)) if (input.kind === 'inline' && input.value.kind === 'text' && byteLength(input.value.text) > session.input_limit_bytes) throw new Error('A connected text input exceeds this daemon’s limit.');
    return { alias: task.id, definition: { ...copy(task.definition), authoring_generation: contract.generation } };
  });
  const onlyStop = !tasks.length && canvas.changes.length === 1 && canvas.changes[0].kind === 'stop' ? canvas.changes[0] : null;
  const request_id = onlyStop ? canvas.details.find((task) => task.id === onlyStop.task)?.stop_request_id ?? crypto.randomUUID() : crypto.randomUUID();
  if (canvas.target.kind === 'existing') return { path: `/v1/workflows/${encodeURIComponent(canvas.target.workflowId)}/changes`, request: { request_id, expected_generation: canvas.target.generation, changes: [...tasks.map((task, index): TaskChangeDto => canvas.tasks[index].version === null ? { kind: 'add', ...task } : { kind: 'revise', task: task.alias, expected_version: canvas.tasks[index].version!, definition: task.definition }), ...copy(canvas.changes)] } };
  if (!tasks.length) throw new Error('Add the first task to this unsaved workflow.');
  const title = canvas.title.trim() || tasks[0].definition.title;
  if (byteLength(title) > 256) throw new Error('Use a workflow title of at most 256 bytes.');
  if (!canvas.target.projectId) throw new Error('Choose a project before starting a workflow. Your drawing is retained.');
  return { path: '/v1/workflows', request: { request_id, project_id: canvas.target.projectId, title, description: '', tasks, inputs: {}, finish_when_settled: canvas.finishWhenSettled, limits: copy(canvas.limits) } };
}

export function cacheTask(canvas: CanvasState, task: TaskReadDto): void {
  const details = [...canvas.details.filter((entry) => entry.id !== task.id), task];
  if (details.length > canvasBounds.tasks || byteLength(JSON.stringify(details)) > canvasBounds.cacheBytes) throw new Error('This canvas has reached its bounded detail cache. Switch task pages or discard cached detail before loading more.');
  canvas.details = details;
}

function position(canvas: CanvasState, id: string, proposed: Point): Point { return canvas.positions[id] ?? proposed; }
export function sourceId(task: string, name: string, binding: InputBinding): string {
  return binding.kind === 'supplied' ? `supplied:${binding.name}` : binding.kind === 'task_output' || binding.kind === 'settled_outcome' ? binding.task : `inline:${task}:${name}`;
}

export function projectGraph(canvas: CanvasState, workflow: WorkflowReadDto | null): { nodes: GraphNode[]; edges: GraphEdge[]; truncated: boolean } {
  const nodes: GraphNode[] = [], edges: GraphEdge[] = [];
  const accepted = workflow?.tasks ?? [];
  const allIds = new Set([...accepted.map((item) => item.id), ...canvas.tasks.map((item) => item.id)]);
  const rowByColumn = new Map<number, number>();
  const columns = new Map<string, number>();
  for (let pass = 0; pass < accepted.length; pass++) for (const task of accepted) if (!columns.has(task.id) && task.dependencies.every((id) => !allIds.has(id) || columns.has(id))) columns.set(task.id, Math.max(1, ...task.dependencies.map((id) => (columns.get(id) ?? 0) + 1)));
  const definitions = accepted.map((summary) => ({ summary, draft: canvas.tasks.find((item) => item.id === summary.id), detail: canvas.details.find((item) => item.id === summary.id) }));
  for (const { summary, draft, detail } of definitions) {
    const column = columns.get(summary.id) ?? 1, row = rowByColumn.get(column) ?? 0;
    rowByColumn.set(column, row + 1);
    nodes.push({ id: summary.id, kind: 'task', label: draft?.definition.title || summary.title, detail: workflow?.wait_question && summary.state === 'queued' ? 'Workflow paused' : readableReason(summary.reason) || (summary.started_at === null ? 'Ready when inputs arrive' : detail?.result?.text?.slice(0, 100) || summary.description || 'Inspect task'), state: summary.state, status: stateLabel(summary).toLowerCase(), draft: !!draft, position: position(canvas, summary.id, { x: column * 310, y: row * 195 + 80 }) });
  }
  for (const task of canvas.tasks.filter((item) => item.version === null)) nodes.push({ id: task.id, kind: 'task', label: task.definition.title || 'New task', detail: task.definition.instructions.slice(0, 100) || 'Describe the work', state: 'draft', draft: true, position: position(canvas, task.id, { x: 340, y: nodes.length * 195 + 80 }) });
  for (const source of canvas.sources) nodes.push({ id: source.id, kind: 'source', label: source.label, detail: source.deferred ? 'Waiting for input' : source.value?.kind === 'uploaded' ? 'Captured file' : source.text.slice(0, 100) || 'Add text', state: source.deferred ? 'waiting' : 'source', draft: true, position: position(canvas, source.id, { x: 0, y: nodes.length * 150 }) });
  const taskDefinitions = [...definitions.map(({ summary, draft, detail }) => ({ id: summary.id, definition: draft?.definition ?? detail?.definition, draft: !!draft, dependencies: summary.dependencies, detail })), ...canvas.tasks.filter((item) => item.version === null).map((task) => ({ id: task.id, definition: task.definition, draft: true, dependencies: task.definition.dependencies, detail: undefined }))];
  for (const task of taskDefinitions) {
    const bound = new Set<string>();
    for (const [name, binding] of Object.entries(task.definition?.inputs ?? {})) {
      const ownSource = canvas.sources.find((item) => item.id === canvas.bindings[`${task.id}:${name}`] || binding.kind === 'supplied' && (item.sharedName ?? item.id) === binding.name);
      const id = ownSource?.id ?? sourceId(task.id, name, binding);
      const sourceTask = binding.kind === 'task_output' || binding.kind === 'settled_outcome';
      if (!nodes.some((node) => node.id === id)) {
        const supplied = binding.kind === 'supplied' ? workflow?.supplied_inputs?.[binding.name] ?? task.detail?.supplied_inputs?.[binding.name] : task.detail?.inline_inputs?.[name];
        const captured = supplied?.value;
        const text = binding.kind === 'inline' && binding.value.kind === 'text' ? binding.value.text : captured?.text;
        const unavailable = captured?.restricted || supplied?.availability === 'restricted' ? 'Access restricted' : supplied?.availability === 'expired' ? 'Input expired' : binding.kind === 'supplied' && !captured ? 'Missing required input' : captured?.kind === 'absent' ? 'Explicitly absent' : captured?.text_truncated ? 'Bounded preview' : '';
        const label = sourceTask ? `Task outside this page` : captured?.name || (binding.kind === 'inline' && binding.value.kind === 'uploaded' ? 'Captured file' : text ? deriveTaskTitle(text).slice(0, 48) : 'Required input');
        nodes.push({ id, kind: sourceTask ? 'boundary' : 'source', label, detail: sourceTask ? 'Load its task page to inspect' : unavailable || text?.slice(0, 100) || 'Text input', state: unavailable ? 'waiting' : 'source', draft: task.draft, position: position(canvas, id, { x: 0, y: nodes.filter((item) => item.kind !== 'task').length * 150 + 30 }) });
      }
      if (sourceTask) bound.add(binding.task);
      edges.push({ id: `edge:${task.id}:${name}`, source: id, target: task.id, input: name, binding, label: binding.kind === 'task_output' ? binding.optional ? 'Result, if available' : 'Successful result' : binding.kind === 'settled_outcome' ? 'Outcome, including failure' : 'Input', draft: task.draft });
    }
    for (const dependency of task.definition?.dependencies ?? task.dependencies) if (!bound.has(dependency)) {
      if (!nodes.some((node) => node.id === dependency)) nodes.push({ id: dependency, kind: 'boundary', label: 'Task outside this page', detail: 'Ordering prerequisite', state: 'source', draft: false, position: position(canvas, dependency, { x: 0, y: nodes.filter((item) => item.kind !== 'task').length * 150 }) });
      edges.push({ id: `order:${dependency}:${task.id}`, source: dependency, target: task.id, input: null, binding: null, label: 'Run after', draft: task.draft });
    }
  }
  return { nodes: nodes.slice(0, canvasBounds.tasks + canvasBounds.sources), edges: edges.slice(0, canvasBounds.edges), truncated: nodes.length > canvasBounds.tasks + canvasBounds.sources || edges.length > canvasBounds.edges };
}
