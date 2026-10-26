import { checkpointState, type ConditionalCheckpoint, type CheckpointState } from './checkpoint';
import { readableReason, stateLabel } from './presentation';
import type { WorkflowTopologyDto, TaskSuppliedInputDto, AuthoringDocumentDto, AuthoringBoundsDto, AuthoringConnectionDto, AuthoringModelDto, RunAuthoringRequest, ChangeWorkflowRequest, CreateWorkflowRequest, InputBinding, InputValue, ProjectDto, TaskCapability, TaskChangeDto, TaskDefinitionDto, TaskReadDto, TaskSessionDto, TaskUploadRequest, WorkflowLimitsDto, WorkflowReadDto } from './protocol.generated';

export const canvasBounds = { changes: 32, tasks: 200, sources: 400, edges: 1024, draftBytes: 2 * 1024 * 1024, cacheBytes: 8 * 1024 * 1024 } as const;
export const capabilityKey = (capability: TaskCapability): string => JSON.stringify([capability.id, capability.operation]);
export const copy = <T>(value: T): T => JSON.parse(JSON.stringify(value));
export const byteLength = (value: string): number => new TextEncoder().encode(value).length;
export type CanvasTarget = { kind: 'new'; projectId?: string } | { kind: 'existing'; workflowId: string; title: string; generation: number; projectId?: string | null };
export interface Point { x: number; y: number }
export function positionCapacity(canvas: CanvasState): void { if (Object.keys(canvas.positions).length >= canvas.bounds.positions) throw new Error(`The canvas has reached its ${canvas.bounds.positions}-card layout bound. Open Workflow and choose Remove off-page positions to free old layout entries. Current cards and pending work keep their positions.`); }
function freePosition(canvas: CanvasState, source: boolean): Point {
  const occupied = Object.values(canvas.positions);
  for (let index = 0; index <= occupied.length; index++) {
    const candidate = { x: source ? 0 : 310 + index % 2 * 310, y: (source ? index : Math.floor(index / 2)) * 195 + 80 };
    if (!occupied.some((point) => Math.abs(point.x - candidate.x) < 270 && Math.abs(point.y - candidate.y) < 175)) return candidate;
  }
  return { x: source ? 0 : 310, y: (occupied.length + 1) * 195 + 80 };
}
export interface CanvasTask { id: string; definition: TaskDefinitionDto; version: number | null; titleEdited: boolean; contractGeneration: number | null }
export type CanvasConnection = AuthoringConnectionDto;
export interface CanvasSource { id: string; label: string; value: InputValue | null; sharedName: string | null; text: string; deferred: boolean }
export interface CanvasState {
  id: string; daemonId: string; actorId: string; target: CanvasTarget; project: ProjectDto | null; disposed: boolean; capacityBlocked: boolean;
  title: string; tasks: CanvasTask[]; sources: CanvasSource[]; changes: TaskChangeDto[]; connections: CanvasConnection[]; usualModel: AuthoringModelDto | null; highlighted: string[]; accepted: Record<string, string>; bounds: AuthoringBoundsDto; authoringSave: CheckpointState<AuthoringDocumentDto>; positionSave: CheckpointState<Record<string, Point>>; checkpoint: ConditionalCheckpoint<AuthoringDocumentDto> | null; positionCheckpoint: ConditionalCheckpoint<Record<string, Point>> | null;
  topology: WorkflowTopologyDto | null; details: TaskReadDto[]; positions: Record<string, Point>; camera: Point & { zoom: number };
  selected: string; inspector: boolean; presentation: 'canvas' | 'list' | 'timeline';
  finishWhenSettled: boolean; limits: WorkflowLimitsDto | null;
  pending: { path: string; request: CreateWorkflowRequest | ChangeWorkflowRequest | RunAuthoringRequest; control?: boolean; run?: boolean } | null;
  pendingUpload: { request: TaskUploadRequest; target: string } | null;
  busy: boolean; error: string; conflict: boolean; notice: string;
}

export interface GraphPort { name: string | null; label: string }
export interface GraphNode { id: string; kind: 'task' | 'source' | 'boundary'; label: string; detail: string; state: string; status?: string; draft: boolean; position: Point; model?: string; inputs?: GraphPort[]; outputs?: GraphPort[] }
export interface GraphEdge { id: string; source: string; target: string; label: string; input: string | null; binding: InputBinding | null; draft: boolean; output?: string | null; unfinished?: boolean; preview?: TaskSuppliedInputDto | null }

export function createCanvas(session: TaskSessionDto, target: CanvasTarget, project: ProjectDto | null = null): CanvasState {
  return { id: crypto.randomUUID(), daemonId: session.daemon_id, actorId: session.actor_id, target: { ...target }, project: project ? copy(project) : null, disposed: false, capacityBlocked: false, title: '', tasks: [], sources: [], changes: [], connections: [], usualModel: null, highlighted: [], accepted: {}, bounds: session.authoring_bounds, authoringSave: checkpointState(), positionSave: checkpointState(), checkpoint: null, positionCheckpoint: null, topology: null, details: [], positions: {}, camera: { x: 50, y: 60, zoom: 1 }, selected: '', inspector: false, presentation: 'canvas', finishWhenSettled: true, limits: null, pending: null, pendingUpload: null, busy: false, error: '', conflict: false, notice: '' };
}

export function canvasDocument(canvas: CanvasState): AuthoringDocumentDto {
  return { tasks: canvas.tasks.map((task) => ({ id: task.id, version: task.version, definition: copy(task.definition), title_edited: task.titleEdited, contract_generation: task.contractGeneration })), sources: canvas.sources.map((source) => ({ id: source.id, label: source.label, value: source.deferred ? null : copy(source.value), shared_name: source.sharedName })), connections: copy(canvas.connections), changes: copy(canvas.changes), usual_model: copy(canvas.usualModel), accepted: { ...canvas.accepted } };
}
export function adoptAuthoring(canvas: CanvasState, document: AuthoringDocumentDto) {
  canvas.tasks = document.tasks.map((task) => ({ id: task.id, version: task.version, definition: copy(task.definition), titleEdited: task.title_edited, contractGeneration: task.contract_generation }));
  canvas.sources = document.sources.map((source) => ({ id: source.id, label: source.label, value: copy(source.value), sharedName: source.shared_name, text: source.value?.kind === 'text' ? source.value.text : '', deferred: source.value === null }));
  canvas.connections = copy(document.connections); canvas.changes = copy(document.changes); canvas.usualModel = copy(document.usual_model); canvas.accepted = { ...document.accepted };
}
export function acceptedId(canvas: CanvasState, id: string): string { return canvas.accepted[id] ?? id; }
export function dependencyIds(dependencies: TaskDefinitionDto['dependencies']): string[] { return Array.isArray(dependencies) ? dependencies : Object.keys(dependencies); }

/** Retire presentation references only; accepted definitions still come from topology. */
export function unusedCanvasReferences(canvas: CanvasState, workflow: WorkflowReadDto | null) {
  if (!workflow) return { wires: [] as string[], sources: [] as string[], aliases: [] as string[], retained: [] as string[] };
  const visible = new Set(workflow?.tasks.map((task) => task.id) ?? []);
  const pending = new Set(canvas.tasks.map((task) => acceptedId(canvas, task.id)));
  const accepted = new Set([...Object.values(canvas.accepted), ...(canvas.topology?.tasks.map((task) => task.id) ?? []), ...canvas.details.map((task) => task.id)]);
  const wires = canvas.connections.filter((wire) => { const target = acceptedId(canvas, wire.target); return accepted.has(target) && !visible.has(target) && !pending.has(target); });
  const removed = new Set(wires.map((wire) => wire.id));
  const remaining = canvas.connections.filter((wire) => !removed.has(wire.id));
  const references = new Set(visible), supplied = new Set<string>();
  function refer(id: string) { references.add(id); references.add(acceptedId(canvas, id)); }
  function definition(value: TaskDefinitionDto) {
    if (value.corrective_of) refer(value.corrective_of);
    for (const id of dependencyIds(value.dependencies)) refer(id);
    for (const input of Object.values(value.inputs)) {
      if (input.kind === 'task_output' || input.kind === 'settled_outcome') refer(input.task);
      else if (input.kind === 'supplied') supplied.add(input.name);
    }
  }
  for (const task of canvas.tasks) { refer(task.id); definition(task.definition); }
  for (const wire of remaining) { refer(wire.source); refer(wire.target); }
  for (const change of canvas.changes) {
    if ('task' in change) refer(change.task);
    if ('definition' in change) definition(change.definition);
    if (change.kind === 'adopt_correction') { refer(change.correction); for (const redirect of change.redirects) refer(redirect.consumer); }
    if (change.kind === 'bind_input') supplied.add(change.name);
  }
  const sources = canvas.sources.filter((source) => wires.some((wire) => wire.source === source.id) && !references.has(source.id) && !(source.sharedName && supplied.has(source.sharedName))).map((source) => source.id);
  const aliases = Object.entries(canvas.accepted).filter(([alias, task]) => !references.has(alias) && !references.has(task)).map(([alias]) => alias);
  return { wires: wires.map((wire) => wire.id), sources, aliases, retained: [...references] };
}

export function removeUnusedCanvasReferences(canvas: CanvasState, workflow: WorkflowReadDto | null) {
  assertEditable(canvas);
  const removal = unusedCanvasReferences(canvas, workflow), wires = new Set(removal.wires), sources = new Set(removal.sources), aliases = new Set(removal.aliases);
  const oldPositions = new Set<string>(removal.sources);
  for (const wire of canvas.connections.filter((wire) => wires.has(wire.id))) for (const id of [wire.source, wire.target]) { oldPositions.add(id); oldPositions.add(acceptedId(canvas, id)); }
  for (const alias of aliases) { oldPositions.add(alias); oldPositions.add(canvas.accepted[alias]); }
  canvas.connections = canvas.connections.filter((wire) => !wires.has(wire.id));
  canvas.sources = canvas.sources.filter((source) => !sources.has(source.id));
  canvas.accepted = Object.fromEntries(Object.entries(canvas.accepted).filter(([alias]) => !aliases.has(alias)));
  const keep = new Set([...removal.retained, ...projectGraph(canvas, workflow).nodes.map((node) => node.id), ...canvas.tasks.map((task) => task.id), ...canvas.sources.map((source) => source.id)]);
  for (const wire of canvas.connections) for (const id of [wire.source, wire.target]) { keep.add(id); keep.add(acceptedId(canvas, id)); }
  canvas.positions = Object.fromEntries(Object.entries(canvas.positions).filter(([id]) => !oldPositions.has(id) || keep.has(id)));
  return removal;
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
  if (canvas.capacityBlocked) throw new Error('This connection holds 8 canvases with unconfirmed edits. Save or resolve one before editing another workflow. Your work remains intact.');
  if (canvas.pending || canvas.pendingUpload || canvas.busy) throw new Error('Recover the pending request before changing this drawing.');
}

export function assertCapacity(canvas: CanvasState, extraChanges = 0): void {
  if (canvas.tasks.length + extraChanges > canvas.bounds.pending_tasks) throw new Error(`This canvas holds at most ${canvas.bounds.pending_tasks} pending tasks. Remove an unused card before adding another.`);
  if (byteLength(JSON.stringify(canvasDocument(canvas))) > canvas.bounds.document_bytes) throw new Error('This drawing has reached its 2 MiB draft limit. Shorten or discard an input before adding more.');
}

export function addCanvasTask(canvas: CanvasState, session: TaskSessionDto, preferred?: TaskCapability): CanvasTask {
  assertEditable(canvas); assertCapacity(canvas, 1);
  positionCapacity(canvas);
  const available = session.capabilities;
  const choice = preferred ?? canvas.usualModel?.capability;
  const capability = available.find((item) => choice && capabilityKey(item) === capabilityKey(choice));
  const id = `draft_${crypto.randomUUID().replaceAll('-', '')}`;
  const task: CanvasTask = { id, version: null, titleEdited: false, contractGeneration: capability ? preferred ? authoring(session, capability)?.generation ?? null : canvas.usualModel?.generation ?? null : null, definition: { title: '', description: '', instructions: '', capability: copy(capability ?? { id: '', operation: '' }), inputs: {}, outputs: {}, dependencies: {}, required: true, corrective_of: null, paths: [], actions: null, tools: [], execution: canvas.project ? { kind: 'project', expected_generation: canvas.project.generation } : { kind: 'legacy' } } };
  const before = canvas.tasks;
  canvas.tasks = [...canvas.tasks, task];
  try { assertCapacity(canvas); } catch (failure) { canvas.tasks = before; throw failure; }
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
  const draft: CanvasTask = { id: task.id, version: task.version, titleEdited: true, contractGeneration: task.definition.authoring_generation ?? authoring(session, task.definition.capability)?.generation ?? null, definition: copy(task.definition) };
  const before = canvas.tasks;
  const sources = copy(canvas.sources), connections = copy(canvas.connections);
  copyTaskConnections(canvas, task, task.id);
  draft.definition.inputs = {}; draft.definition.dependencies = {};
  canvas.tasks = [...canvas.tasks, draft];
  try { assertCapacity(canvas); } catch (failure) { canvas.tasks = before; canvas.sources = sources; canvas.connections = connections; throw failure; }
  return canvas.tasks.find((item) => item.id === draft.id)!;
}

/** Retain the daemon's projected legacy predicates before clearing the accepted input fields. */
export function copyTaskConnections(canvas: CanvasState, task: TaskReadDto, target: string) {
  const topology = canvas.topology?.tasks.find((item) => item.id === task.id && item.version === task.version);
  if (!topology) throw new Error('Refresh this task’s connections before editing. Its current definition is unchanged.');
  const prior = canvas.connections.filter((wire) => acceptedId(canvas, wire.target) === task.id);
  if (target === task.id) canvas.connections = canvas.connections.filter((wire) => acceptedId(canvas, wire.target) !== task.id);
  for (const [input, binding] of Object.entries(task.definition.inputs)) {
    let source: string, output: string | null = null, optional = false, outcome = false;
    if (binding.kind === 'task_output' || binding.kind === 'settled_outcome') { source = binding.task; if (binding.kind === 'task_output') { output = binding.output; optional = binding.optional; } else outcome = true; }
    else {
      const retained = prior.find((wire) => wire.kind === 'data' && wire.input === input && matchesBinding(canvas, wire, binding));
      source = retained?.source ?? sourceId(task.id, input, binding);
      if (!canvas.sources.some((item) => item.id === source)) canvas.sources.push({ id: source, label: binding.kind === 'supplied' ? binding.name : task.inline_inputs[input]?.value?.name ?? 'Task input', value: binding.kind === 'inline' ? copy(binding.value) : null, sharedName: binding.kind === 'supplied' ? binding.name : null, text: binding.kind === 'inline' && binding.value.kind === 'text' ? binding.value.text : '', deferred: binding.kind === 'supplied' });
      optional = binding.kind === 'supplied' && binding.optional;
    }
    canvas.connections.push({ id: `edge:${target}:${input}`, source, target, input, output, kind: 'data', optional, outcome });
  }
  for (const [source, condition] of Object.entries(topology.dependencies)) canvas.connections.push({ id: `order:${source}:${target}`, source, target, input: null, output: null, kind: condition === 'settled' ? 'after_completion' : 'after_success', optional: false, outcome: false });
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
  if (canvas.sources.length >= canvas.bounds.sources) throw new Error('This canvas has reached its 400-source limit. Discard an unused source before adding another.');
  const id = `source_${crypto.randomUUID().replaceAll('-', '')}`;
  const source = { id, label, value, sharedName: value === null ? id : null, text: value?.kind === 'text' ? value.text : '', deferred: value === null };
  const before = canvas.sources;
  canvas.sources = [...canvas.sources, source];
  try { assertCapacity(canvas); } catch (failure) { canvas.sources = before; throw failure; }
  canvas.positions[id] = freePosition(canvas, true); canvas.selected = id; canvas.inspector = true;
  return canvas.sources.find((item) => item.id === id)!;
}

export function useUsualModel(canvas: CanvasState, session: TaskSessionDto, key: string, includeUnassigned: boolean) {
  assertEditable(canvas);
  const capability = session.capabilities.find((item) => capabilityKey(item) === key);
  const contract = capability && authoring(session, capability);
  if (!capability || !contract) throw new Error('Choose an available model or agent.');
  canvas.usualModel = { capability: copy(capability), generation: contract.generation };
  canvas.highlighted = includeUnassigned ? canvas.tasks.filter((task) => task.version === null && !task.definition.capability.id).map((task) => task.id) : [];
  for (const task of canvas.tasks.filter((item) => canvas.highlighted.includes(item.id))) chooseCapability(task, session, key);
  resolveConnections(canvas, session);
}

/** Resolve only actual single choices or advertised defaults; ambiguous intent remains visible. */
export function resolveConnections(canvas: CanvasState, session: TaskSessionDto) {
  for (const wire of canvas.connections.filter((item) => item.kind === 'data')) {
    const producer = canvas.tasks.find((item) => item.id === wire.source) ?? canvas.details.find((item) => item.id === wire.source);
    const consumer = canvas.tasks.find((item) => item.id === wire.target);
    const from = producer && authoring(session, producer.definition.capability), to = consumer && authoring(session, consumer.definition.capability);
    if (!wire.output && from && !wire.outcome) wire.output = from.default_output ?? (from.outputs.length === 1 ? from.outputs[0].name : null);
    if (!wire.input && to) {
      const occupied = new Set(canvas.connections.filter((item) => item.id !== wire.id && item.target === wire.target).map((item) => item.input));
      const slots = to.inputs.filter((item) => !occupied.has(item.name));
      if (slots.length === 1 && !to.allows_additional_inputs) wire.input = slots[0].name;
      else if (!slots.length && to.allows_additional_inputs) wire.input = `input_${wire.id}`;
    }
  }
}

export function addConnection(canvas: CanvasState, session: TaskSessionDto, source: string, target: string, output: string | null = null, input: string | null = null, kind: CanvasConnection['kind'] = 'data') {
  assertEditable(canvas);
  if (source === target) throw new Error('A task cannot connect to itself.');
  if (canvas.connections.length >= canvas.bounds.connections) throw new Error('This canvas has reached its connection bound. Remove an unused wire first.');
  const wire: CanvasConnection = { id: crypto.randomUUID().replaceAll('-', ''), source, target, output, input, kind, optional: false, outcome: false };
  canvas.connections = [...canvas.connections, wire]; resolveConnections(canvas, session); assertCapacity(canvas);
  return canvas.connections.find((item) => item.id === wire.id)!;
}

/** The source owns its binding value; generated keys have no authoring UI. */
export function sourceBinding(source: CanvasSource): InputBinding {
  return source.deferred || source.sharedName ? { kind: 'supplied', name: source.sharedName ?? source.id, optional: false } : { kind: 'inline', value: copy(source.value ?? { kind: 'text', text: source.text }) };
}

export function updateSource(canvas: CanvasState, source: CanvasSource, text: string, limit: number, deferred = source.deferred): void {
  assertEditable(canvas);
  if (byteLength(text) > limit) throw new Error(`This input exceeds the ${limit.toLocaleString()} byte limit. Its previous content is intact.`);
  const previous = copy(source);
  source.text = text; source.value = { kind: 'text', text }; source.deferred = deferred; source.sharedName = deferred ? source.sharedName ?? source.id : null;
  source.label = deriveTaskTitle(text).slice(0, 48) || 'Text input';
  try { assertCapacity(canvas); } catch (failure) { Object.assign(source, previous); throw failure; }
}

export function stageChange(canvas: CanvasState, change: TaskChangeDto): void {
  assertEditable(canvas); assertCapacity(canvas, 1);
  const before = canvas.changes; canvas.changes = [...canvas.changes, copy(change)];
  try { assertCapacity(canvas); } catch (failure) { canvas.changes = before; throw failure; }
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

/** A saved wire may name an accepted source only while its exact binding still matches. */
function matchesBinding(canvas: CanvasState, wire: CanvasConnection, binding: InputBinding): boolean {
  if (wire.kind !== 'data') return false;
  const source = canvas.sources.find((item) => item.id === wire.source);
  if (binding.kind === 'settled_outcome') return wire.outcome && acceptedId(canvas, wire.source) === binding.task;
  if (wire.outcome) return false;
  if (binding.kind === 'task_output') return !source && acceptedId(canvas, wire.source) === binding.task && wire.output === binding.output && wire.optional === binding.optional;
  if (!source) return false;
  const saved = sourceBinding(source);
  return binding.kind === 'supplied' ? saved.kind === 'supplied' && saved.name === binding.name && wire.optional === binding.optional : saved.kind === 'inline' && (saved.value.kind === 'text' && binding.value.kind === 'text' ? saved.value.text === binding.value.text : saved.value.kind === 'uploaded' && binding.value.kind === 'uploaded' && saved.value.input_id === binding.value.input_id);
}
function presentationSource(canvas: CanvasState, task: WorkflowTopologyDto['tasks'][number], name: string): string | null {
  const input = task.inputs[name];
  const detail = canvas.details.find((item) => item.id === task.id && item.version === task.version);
  const binding: InputBinding | null = detail?.definition.inputs[name] ?? (input.kind === 'task_output' ? { kind: 'task_output', task: input.source, output: input.output!, optional: input.optional } : input.kind === 'settled_outcome' ? { kind: 'settled_outcome', task: input.source } : input.kind === 'supplied' ? { kind: 'supplied', name: input.source.slice('supplied:'.length), optional: input.optional } : input.preview?.availability === 'available' && !input.preview.value?.restricted ? input.preview.value?.artifact_id ? { kind: 'inline', value: { kind: 'uploaded', input_id: input.preview.value.artifact_id } } : input.preview.value?.text !== null && input.preview.value?.text !== undefined && !input.preview.value.text_truncated ? { kind: 'inline', value: { kind: 'text', text: input.preview.value.text } } : null : null);
  if (!binding) return null;
  return canvas.connections.find((wire) => acceptedId(canvas, wire.target) === task.id && wire.input === name && matchesBinding(canvas, wire, binding))?.source ?? null;
}

export function projectGraph(canvas: CanvasState, workflow: WorkflowReadDto | null): { nodes: GraphNode[]; edges: GraphEdge[]; truncated: boolean } {
  const nodes: GraphNode[] = [], edges: GraphEdge[] = [];
  const accepted = workflow?.tasks ?? [];
  for (const [index, summary] of accepted.entries()) {
    const draft = canvas.tasks.find((item) => item.id === summary.id), detail = canvas.details.find((item) => item.id === summary.id);
    nodes.push({ id: summary.id, kind: 'task', label: draft?.definition.title || summary.title, detail: workflow?.wait_question && summary.state === 'queued' ? 'Workflow paused' : readableReason(summary.reason) || detail?.result?.text?.slice(0, 100) || summary.description || 'Inspect task', state: summary.state, status: stateLabel(summary).toLowerCase(), draft: !!draft, position: position(canvas, summary.id, { x: 310 + index % 3 * 310, y: Math.floor(index / 3) * 205 + 80 }) });
  }
  for (const task of canvas.tasks.filter((item) => !accepted.some((summary) => summary.id === item.id))) nodes.push({ id: task.id, kind: 'task', label: task.definition.title || 'New task', detail: task.definition.instructions.slice(0, 100) || 'Click to define this task', state: 'draft', draft: true, position: position(canvas, task.id, { x: 340, y: nodes.length * 205 + 80 }) });
  for (const source of canvas.sources) nodes.push({ id: source.id, kind: 'source', label: source.label, detail: source.deferred ? source.sharedName ? 'Shared input' : 'Waiting for input' : source.value?.kind === 'uploaded' ? 'Captured file' : source.text.slice(0, 100) || 'Add text', state: source.deferred ? 'waiting' : 'source', draft: true, position: position(canvas, source.id, { x: 0, y: nodes.length * 150 }) });
  function external(id: string, preview: TaskSuppliedInputDto | null = null, source = false) {
    if (nodes.some((item) => item.id === id)) return;
    const captured = preview?.value;
    const unavailable = preview?.availability === 'restricted' || captured?.restricted ? 'Access restricted' : preview?.availability === 'expired' ? 'Input expired' : preview?.availability === 'missing' ? 'Missing required input' : '';
    nodes.push({ id, kind: source ? 'source' : 'boundary', label: source ? captured?.name || (captured?.text ? deriveTaskTitle(captured.text).slice(0, 48) : 'Task input') : 'Task outside this page', detail: source ? unavailable || captured?.text?.slice(0, 100) || 'Inspect input' : 'Load its task page to inspect', state: unavailable ? 'waiting' : 'source', draft: false, position: position(canvas, id, { x: 0, y: nodes.filter((item) => item.kind !== 'task').length * 150 + 30 }) });
  }
  for (const task of canvas.topology?.tasks ?? []) {
    if (!accepted.some((summary) => summary.id === task.id) || canvas.tasks.some((item) => item.id === task.id)) continue;
    for (const [name, input] of Object.entries(task.inputs)) {
      const isSource = input.kind === 'inline' || input.kind === 'supplied';
      const source = presentationSource(canvas, task, name) ?? input.source;
      external(source, input.preview, isSource);
      const sourceNode = nodes.find((item) => item.id === source);
      if (sourceNode && isSource) sourceNode.draft = false;
      if (sourceNode && isSource && input.kind === 'supplied') {
        const unavailable = input.preview?.availability === 'restricted' || input.preview?.value?.restricted ? 'Access restricted' : input.preview?.availability === 'expired' ? 'Input expired' : input.preview?.availability === 'missing' ? 'Missing required input' : '';
        sourceNode.detail = unavailable || input.preview?.value?.text?.slice(0, 100) || 'Shared input';
        if (unavailable && unavailable !== 'Missing required input') sourceNode.label = 'Task input';
        else if (input.preview?.value?.text) sourceNode.label = deriveTaskTitle(input.preview.value.text).slice(0, 48);
      }
      const binding: InputBinding | null = input.kind === 'task_output' ? { kind: 'task_output', task: input.source, output: input.output!, optional: input.optional } : input.kind === 'settled_outcome' ? { kind: 'settled_outcome', task: input.source } : input.kind === 'supplied' ? { kind: 'supplied', name: input.source.slice('supplied:'.length), optional: input.optional } : null;
      edges.push({ id: `edge:${task.id}:${name}`, source: acceptedId(canvas, source), target: task.id, input: name, output: input.output, binding, preview: input.preview, label: input.kind === 'settled_outcome' ? 'Outcome, including failure' : isSource ? 'Input' : input.optional ? 'Result, if available' : 'Successful result', draft: false });
    }
    for (const [source, condition] of Object.entries(task.dependencies)) {
      external(source);
      edges.push({ id: `order:${source}:${task.id}`, source, target: task.id, input: null, binding: null, label: `${condition === 'settled' ? 'After completion' : 'After success'}${task.dependencies_legacy ? ' · Retained ordering' : ''}`, draft: false });
    }
  }
  for (const wire of canvas.connections) {
    const targetId = acceptedId(canvas, wire.target), sourceId = acceptedId(canvas, wire.source);
    if (!canvas.tasks.some((task) => task.id === targetId)) continue;
    const source = canvas.sources.find((item) => item.id === wire.source);
    external(sourceId); external(targetId);
    const unfinished = wire.kind === 'data' && (!wire.input || (!source && !wire.outcome && !wire.output));
    edges.push({ id: wire.id, source: sourceId, target: targetId, input: wire.input, output: wire.output, binding: wire.kind !== 'data' ? null : source ? sourceBinding(source) : wire.outcome ? { kind: 'settled_outcome', task: sourceId } : wire.output ? { kind: 'task_output', task: sourceId, output: wire.output, optional: wire.optional } : null, draft: true, unfinished, label: unfinished ? !wire.output && !source ? 'Choose output' : 'Choose input' : wire.kind === 'after_completion' ? 'After completion' : wire.kind === 'after_success' ? 'After success' : source ? 'Input' : wire.outcome ? 'Outcome, including failure' : wire.optional ? 'Result, if available' : 'Successful result' });
  }
  return { nodes: nodes.slice(0, canvasBounds.tasks + canvasBounds.sources), edges: edges.slice(0, canvasBounds.edges), truncated: nodes.length > canvasBounds.tasks + canvasBounds.sources || edges.length > canvasBounds.edges };
}
