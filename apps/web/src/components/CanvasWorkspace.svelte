<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import type { InputBinding, TaskCapability, TaskChangeDto, TaskReadDto, TaskSessionDto, TaskUploadReceipt, WorkflowLimitsDto, WorkflowReadDto, WorkflowReceipt } from '../lib/protocol.generated';
  import { addCanvasTask, addSource, assertCapacity, assertEditable, authoring, cacheTask, capabilityKey, changeCount, chooseCapability, connectBinding, copy, freezeCanvas, hasChanges, inputSlots, projectGraph, removeConnection, reviseTask, sourceBinding, stageChange, synchronizeDependencies, updateDefinition, updateInstructions, updateSource, type CanvasState, type CanvasTask, type GraphEdge } from '../lib/canvas';
  import { DaemonClient, DaemonError } from '../lib/transport';
  import { captureTextRequest } from '../lib/task-input';
  import { executionLanes, readableReason, saveBlob, stateLabel, timeLabel } from '../lib/presentation';
  import WorkflowCanvas from './WorkflowCanvas.svelte';
  import TaskPermissions from './TaskPermissions.svelte';
  import '../canvas.css';

  let { canvas, workflow, session, client, preferredCapability, onaccepted, onreviewed, onrefresh, onpage, onerror, oncapabilitychoice, onmodels, ondiscard }: {
    canvas: CanvasState; workflow: WorkflowReadDto | null; session: TaskSessionDto; client: DaemonClient;
    preferredCapability?: TaskCapability; onaccepted: (receipt: WorkflowReceipt) => Promise<void>;
    onreviewed: (workflow: WorkflowReadDto) => void; onrefresh: () => Promise<void>;
    onpage: (cursor?: string) => Promise<void>; onerror: (failure: unknown) => void;
    oncapabilitychoice: (capability: TaskCapability) => void; onmodels: () => void; ondiscard: () => void;
  } = $props();
  let graph = $derived(projectGraph(canvas, workflow));
  let draftTask = $derived(canvas.tasks.find((task) => task.id === canvas.selected));
  let selectedTask = $derived(canvas.details.find((task) => task.id === canvas.selected));
  let selectedSource = $derived(canvas.sources.find((source) => source.id === canvas.selected));
  let selectedNode = $derived(graph.nodes.find((node) => node.id === canvas.selected));
  let selectedEdges = $derived(graph.edges.filter((edge) => edge.target === canvas.selected));
  let sourceEdges = $derived(graph.edges.filter((edge) => edge.source === canvas.selected));
  let contract = $derived(draftTask ? authoring(session, draftTask.definition.capability) : selectedTask ? authoring(session, selectedTask.definition.capability) : undefined);
  let connection = $state<{ source: string; target: string; output: string; slot: string; outcome: boolean; optional: boolean; order: boolean } | null>(null);
  let edge = $state<GraphEdge | null>(null);
  let supplyText = $state('');
  let waitQuestion = $state('');
  let pauseOpen = $state(false);
  let limitsOpen = $state(false);
  let limits = $state<WorkflowLimitsDto | null>(null);
  let correction = $state('');
  let redirectChoices = $state<string[]>([]);
  let correctionOpen = $state(false);
  let resultOpen = $state(false);
  let loading = $state(false);
  let instructionsElement: HTMLTextAreaElement | undefined = $state();
  let attachment: HTMLInputElement;
  let attachmentTarget = '';
  let inspector: HTMLElement | undefined = $state();
  let mounted = true;
  let loadGeneration = 0;
  const limitLabels: Record<keyof WorkflowLimitsDto, string> = { tasks: 'Total tasks', concurrent_tasks: 'Tasks at once', model_batches: 'Follow-up batches', rejected_responses: 'Consecutive refused proposals', corrective_failures: 'Failed corrections in a row', active_execution_ms: 'Execution time (milliseconds)', tool_calls_per_task: 'Tool calls per task' };
  onDestroy(() => { mounted = false; loadGeneration++; });

  $effect(() => {
    const snapshot = workflow;
    const signature = snapshot?.tasks.map((task) => `${task.id}:${task.version}:${task.state}:${task.has_result}:${task.stop_requested}:${task.reason}:${task.started_at}:${task.ended_at}`).join('|');
    if (snapshot && signature) untrack(() => void loadPage(snapshot));
  });
  $effect(() => {
    const initial = graph.nodes.filter((node) => !canvas.positions[node.id]).map((node) => ({ id: node.id, position: node.position }));
    untrack(() => { for (const node of initial) if (Object.keys(canvas.positions).length < 600) canvas.positions[node.id] = node.position; });
  });

  async function loadPage(snapshot: WorkflowReadDto) {
    const generation = ++loadGeneration;
    loading = true;
    const queue = snapshot.tasks.filter((task) => !canvas.details.some((known) => known.id === task.id && known.version === task.version && known.state === task.state && Boolean(known.result || known.result_restricted) === task.has_result && known.stop_requested === task.stop_requested && known.reason === task.reason && known.started_at === task.started_at && known.ended_at === task.ended_at));
    try {
      await Promise.all(Array.from({ length: Math.min(4, queue.length) }, async () => {
        while (queue.length && mounted && !canvas.disposed && generation === loadGeneration) {
          const task = queue.shift()!;
          const detail = await client.request<TaskReadDto>(`/v1/workflows/${encodeURIComponent(snapshot.id)}/tasks/${encodeURIComponent(task.id)}`);
          if (mounted && !canvas.disposed && generation === loadGeneration) cacheTask(canvas, detail);
        }
      }));
    } catch (failure) { if (mounted && !canvas.disposed && generation === loadGeneration) report(failure); }
    finally { if (generation === loadGeneration) loading = false; }
  }

  function report(failure: unknown) { canvas.error = failure instanceof Error ? failure.message : String(failure); onerror(failure); }
  function attempt(action: () => void) { try { canvas.error = ''; canvas.notice = ''; action(); } catch (failure) { report(failure); } }
  function editable(id: string): CanvasTask {
    assertEditable(canvas);
    const draft = canvas.tasks.find((item) => item.id === id);
    if (draft) return draft;
    const task = canvas.details.find((item) => item.id === id);
    if (!task) throw new Error('Load this task’s details before changing it. Tasks outside this page remain intact.');
    if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation;
    return reviseTask(canvas, task, session);
  }

  export async function addTask() {
    attempt(() => { if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation; addCanvasTask(canvas, session, preferredCapability); connection = null; edge = null; });
    await tick(); instructionsElement?.focus();
  }
  export function canNavigate() { return true; }

  function select(id: string) {
    canvas.selected = id; canvas.inspector = true; connection = null; edge = null; correctionOpen = false; resultOpen = false; supplyText = '';
    if (workflow?.tasks.some((task) => task.id === id) && !canvas.details.some((task) => task.id === id)) {
      const snapshot = workflow;
      void client.request<TaskReadDto>(`/v1/workflows/${encodeURIComponent(snapshot.id)}/tasks/${encodeURIComponent(id)}`).then((task) => { if (mounted && !canvas.disposed) cacheTask(canvas, task); }).catch((failure) => { if (mounted && !canvas.disposed) report(failure); });
    }
  }
  async function closeInspector() { canvas.inspector = false; connection = null; edge = null; await tick(); document.querySelector<HTMLButtonElement>(`[data-node="${CSS.escape(canvas.selected)}"] .node-content`)?.focus(); }

  function sourcePorts(id: string) {
    const draft = canvas.tasks.find((task) => task.id === id);
    const actual = canvas.details.find((task) => task.id === id);
    const metadata = authoring(session, draft?.definition.capability ?? actual?.definition.capability ?? { id: '', operation: '' });
    if (draft?.version === null) return metadata?.outputs ?? [];
    const declarations = actual?.definition.outputs ?? {};
    const published = actual?.result?.outputs ?? {};
    return [...new Set([...Object.keys(declarations), ...Object.keys(published)])].map((name) => ({ name, label: `${metadata?.outputs.find((port) => port.name === name)?.label ?? name.replaceAll('_', ' ')}${actual?.result && !published[name] ? ' · Not published' : ''}`, required: declarations[name]?.required ?? true, media_type: declarations[name]?.media_type ?? published[name]?.media_type ?? null, max_bytes: declarations[name]?.max_bytes ?? session.task_output_limit_bytes, available: actual?.result_restricted ? false : published[name]?.available ?? (declarations[name]?.required === false || !actual?.result) }));
  }

  const connectionPorts = $derived(connection ? sourcePorts(connection.source) : []);
  const targetDraft = $derived(connection ? canvas.tasks.find((task) => task.id === connection!.target) : undefined);
  const targetContract = $derived(targetDraft ? authoring(session, targetDraft.definition.capability) : undefined);
  const availableSlots = $derived(targetContract?.inputs.filter((slot) => !(slot.name in (targetDraft?.definition.inputs ?? {}))) ?? []);
  const outputPlan = $derived(connection && !connection.outcome && !connection.order ? draftOutputPlan(connection.source, connection.output) : null);

  function draftOutputPlan(id: string, name: string) {
    const producer = canvas.tasks.find((task) => task.id === id && task.version === null);
    if (!producer || !name) return null;
    const metadata = authoring(session, producer.definition.capability);
    const names = [...new Set([...Object.keys(producer.definition.outputs), name])];
    if (!metadata || !session.task_output_limit_bytes) return null;
    const share = Math.floor(session.task_output_limit_bytes / names.length);
    if (share < 1 || names.some((key) => !metadata.outputs.some((port) => port.name === key))) return null;
    return Object.fromEntries(names.map((key) => {
      const port = metadata.outputs.find((item) => item.name === key)!;
      return [key, { required: producer.definition.outputs[key]?.required ?? port.required, media_type: port.media_type, max_bytes: Math.min(share, port.max_bytes, producer.definition.outputs[key]?.max_bytes ?? port.max_bytes) }];
    }));
  }

  function beginConnection(source: string, target = '', order = false) {
    attempt(() => {
      if (canvas.pending || canvas.pendingUpload || canvas.busy) throw new Error('Recover this pending submission before changing connections.');
      if (target) editable(target);
      const ports = sourcePorts(source);
      const task = canvas.tasks.find((item) => item.id === source) ?? canvas.details.find((item) => item.id === source);
      const metadata = task ? authoring(session, task.definition.capability) : undefined;
      connection = { source, target, output: ports.length === 1 ? ports[0].name : metadata?.default_output ?? '', slot: '', outcome: false, optional: false, order };
      edge = null; canvas.inspector = true;
      if (target) chooseDestination(target);
    });
  }

  function chooseDestination(id: string) {
    attempt(() => {
      if (!connection) return;
      const task = editable(id); connection.target = id;
      const { slots, additional } = inputSlots(task, session);
      connection.slot = !additional && slots.length === 1 ? slots[0].name : '';
    });
  }

  function pruneUnusedResults(task: CanvasTask) {
    const metadata = authoring(session, task.definition.capability);
    if (!metadata) return;
    const outputs = { ...task.definition.outputs };
    for (const name of Object.keys(outputs)) {
      const used = canvas.tasks.some((consumer) => Object.values(consumer.definition.inputs).some((input) => input.kind === 'task_output' && input.task === task.id && input.output === name));
      if (!used && !metadata.outputs.some((port) => port.name === name)) delete outputs[name];
    }
    task.definition.outputs = outputs;
  }

  function confirmConnection() {
    attempt(() => {
      if (!connection?.source || !connection.target) throw new Error('Choose the task that should receive this input.');
      if (connection.source === connection.target) throw new Error('A task cannot receive its own result.');
      const beforeTasks = copy(canvas.tasks), beforeBindings = { ...canvas.bindings };
      try {
      const task = editable(connection.target);
      if (connection.order) { task.explicitDependencies = [...new Set([...task.explicitDependencies, connection.source])]; synchronizeDependencies(task); }
      else {
        let binding: InputBinding;
        let producer: CanvasTask | undefined;
        let allocated: ReturnType<typeof draftOutputPlan> = null;
        const source = canvas.sources.find((item) => item.id === connection!.source);
        if (source) binding = sourceBinding(source);
        else if (graph.nodes.find((node) => node.id === connection!.source)?.kind === 'source') {
          const existing = graph.edges.find((item) => item.source === connection!.source)?.binding;
          if (!existing || existing.kind !== 'inline' && existing.kind !== 'supplied') throw new Error('This input is unavailable. Reload its source before connecting it.');
          if (graph.nodes.find((node) => node.id === connection!.source)?.detail === 'Access restricted' || graph.nodes.find((node) => node.id === connection!.source)?.detail === 'Input expired') throw new Error('This input is restricted or expired. Select an available source.');
          binding = copy(existing);
        }
        else if (connection.outcome) binding = { kind: 'settled_outcome', task: connection.source };
        else {
          const port = sourcePorts(connection.source).find((item) => item.name === connection!.output);
          if (!port || 'available' in port && !port.available) throw new Error('Choose an available, real result from this task.');
          binding = { kind: 'task_output', task: connection.source, output: port.name, optional: connection.optional };
          producer = canvas.tasks.find((item) => item.id === connection!.source && item.version === null);
          if (producer) { allocated = draftOutputPlan(producer.id, port.name); if (!allocated) throw new Error('The result allowance is unavailable or an earlier connection no longer matches this agent. Refresh Models & agents and reconnect its results.'); }
        }
        connectBinding(canvas, task, binding, session, connection.slot || undefined, source?.id);
        if (producer && allocated) producer.definition.outputs = allocated;
      }
      assertCapacity(canvas);
      const target = connection.target;
      canvas.notice = `Connected to ${task.definition.title || 'New task'}. Apply the drawing when it is ready.`;
      connection = null; select(target);
      } catch (failure) { canvas.tasks = beforeTasks; canvas.bindings = beforeBindings; throw failure; }
    });
  }

  function addText(target = '', missing = false) {
    attempt(() => {
      if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation;
      const source = addSource(canvas, missing ? 'Required input' : 'Text input', missing ? null : { kind: 'text', text: '' });
      if (target) {
        const task = editable(target), slots = inputSlots(task, session);
        if (slots.additional || slots.slots.length === 1) connectBinding(canvas, task, sourceBinding(source), session, slots.additional ? undefined : slots.slots[0].name, source.id);
        else beginConnection(source.id, target);
      }
    });
  }

  async function captureFile(file?: File, target = '') {
    const current = canvas, connectionClient = client, reportError = onerror;
    if (current.busy || current.disposed || current.pending || file && current.pendingUpload) return;
    if (file && current.target.kind === 'existing' && !hasChanges(current) && workflow) current.target.generation = workflow.generation;
    current.busy = true; current.error = ''; current.notice = '';
    try {
      if (file) current.pendingUpload = { request: await captureTextRequest(file, session.input_limit_bytes), target };
      if (current.disposed || !current.pendingUpload) return;
      const pending = current.pendingUpload;
      const receipt = await connectionClient.request<TaskUploadReceipt>('/v1/task-inputs', pending.request);
      if (current.disposed) return;
      current.pendingUpload = null; current.busy = false;
      let source;
      try { source = addSource(current, receipt.name, { kind: 'uploaded', input_id: receipt.input_id }); }
      catch (failure) { current.pendingUpload = pending; throw failure; }
      if (pending.target) {
        const task = editable(pending.target), slots = inputSlots(task, session);
        if (slots.additional || slots.slots.length === 1) connectBinding(current, task, sourceBinding(source), session, slots.additional ? undefined : slots.slots[0].name, source.id);
        else if (mounted) beginConnection(source.id, pending.target);
      }
    } catch (failure) {
      if (current.disposed) return;
      current.error = failure instanceof Error ? failure.message : 'The file capture could not be confirmed.';
      if (failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) current.pendingUpload = null;
      reportError(failure);
    } finally { if (!current.disposed) current.busy = false; }
  }

  function attachFile(target = '') { attachmentTarget = target; attachment.click(); }

  async function submit(receiptOnly = false) {
    const current = canvas, connectionClient = client, accept = onaccepted, reportError = onerror;
    if (current.disposed || current.busy || current.pendingUpload || current.conflict) return;
    current.error = ''; current.busy = true;
    try {
      if (!current.pending) {
        const freshSession = await connectionClient.request<TaskSessionDto>('/v1/session');
        if (current.disposed) return;
        if (freshSession.daemon_id !== current.daemonId || freshSession.actor_id !== current.actorId) throw new DaemonError('unauthorized', 'The authenticated daemon or actor changed. Reconnect before authoring work.', 401);
        current.pending = freezeCanvas(current, freshSession);
      }
      const pending = current.pending;
      const receipt = await connectionClient.request<WorkflowReceipt>(receiptOnly ? `/v1/workflow-requests/${encodeURIComponent(pending.request.request_id)}` : pending.path, receiptOnly ? undefined : pending.request);
      if (current.disposed) return;
      for (const [alias, id] of Object.entries(receipt.aliases)) if (current.positions[alias] && alias !== id) { current.positions[id] = current.positions[alias]; delete current.positions[alias]; }
      current.pending = null;
      await accept(receipt);
    } catch (failure) {
      if (current.disposed) return;
      current.error = failure instanceof Error ? failure.message : 'The daemon reply could not be confirmed.';
      if (!receiptOnly && failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) { current.pending = null; current.conflict = current.target.kind === 'existing' && failure.status === 409; }
      reportError(failure);
    } finally { if (!current.disposed) current.busy = false; }
  }

  async function refreshConflict() {
    const current = canvas;
    if (current.target.kind !== 'existing' || current.busy) return;
    current.busy = true;
    try {
      const snapshot = await client.request<WorkflowReadDto>(`/v1/workflows/${encodeURIComponent(current.target.workflowId)}`);
      if (current.disposed || current.target.kind !== 'existing') return;
      for (const draft of current.tasks.filter((task) => task.version !== null)) {
        const actual = await client.request<TaskReadDto>(`/v1/workflows/${encodeURIComponent(snapshot.id)}/tasks/${encodeURIComponent(draft.id)}`);
        if (current.disposed) return;
        cacheTask(current, actual);
        if (actual.started_at !== null || actual.state !== 'queued') throw new Error(`${actual.definition.title} has started. Discard its proposed edit or create a correction; the drawing is retained.`);
        if (actual.version !== draft.version) throw new Error(`${actual.definition.title} was edited elsewhere. Discard this task’s proposed edit and review its latest connections before editing again.`);
      }
      current.target.generation = snapshot.generation; current.conflict = false; current.error = ''; onreviewed(snapshot);
    } catch (failure) { if (!current.disposed) report(failure); }
    finally { if (!current.disposed) current.busy = false; }
  }

  function stage(change: TaskChangeDto) { attempt(() => { if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation; stageChange(canvas, change); canvas.notice = 'Change added to the drawing. Apply changes to send it.'; }); }
  function discardTask(task: CanvasTask) {
    attempt(() => {
      assertEditable(canvas);
      if (task.version === null && canvas.tasks.some((item) => item.id !== task.id && item.definition.dependencies.includes(task.id))) throw new Error('Disconnect this draft’s consumers before discarding it.');
      canvas.tasks = canvas.tasks.filter((item) => item.id !== task.id);
      if (task.version === null) delete canvas.positions[task.id];
      for (const key of Object.keys(canvas.bindings)) if (key.startsWith(`${task.id}:`)) delete canvas.bindings[key];
      canvas.inspector = false; canvas.selected = ''; canvas.conflict = false;
    });
  }

  function inspectEdge(value: GraphEdge) { edge = value; connection = null; canvas.inspector = true; }
  function deleteEdge() {
    attempt(() => {
      if (!edge) return;
      const task = editable(edge.target), binding = edge.binding;
      if (edge.input) { removeConnection(task, edge.input); delete canvas.bindings[`${task.id}:${edge.input}`]; }
      else { task.explicitDependencies = task.explicitDependencies.filter((id) => id !== edge!.source); synchronizeDependencies(task); }
      if (binding?.kind === 'task_output') {
        const producer = canvas.tasks.find((item) => item.id === binding.task && item.version === null);
        if (producer && !canvas.tasks.some((item) => Object.values(item.definition.inputs).some((input) => input.kind === 'task_output' && input.task === producer.id && input.output === binding.output))) {
          const outputs = { ...producer.definition.outputs }; delete outputs[binding.output]; producer.definition.outputs = outputs;
        }
      }
      edge = null; canvas.notice = 'Connection removed from the drawing. Existing ordering prerequisites remain until you remove their Run after connection.';
    });
  }

  function createCorrection(task: TaskReadDto) {
    attempt(() => {
      if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation;
      const selected = canvas.selected, inspectorOpen = canvas.inspector;
      const draft = addCanvasTask(canvas, session, task.definition.capability);
      try {
      draft.definition = { ...copy(task.definition), title: `Correction: ${task.definition.title}`, instructions: '', corrective_of: task.id };
      draft.titleEdited = true; draft.explicitDependencies = [...task.definition.dependencies];
      assertCapacity(canvas);
      canvas.selected = draft.id;
      } catch (failure) { canvas.tasks = canvas.tasks.filter((item) => item.id !== draft.id); delete canvas.positions[draft.id]; canvas.selected = selected; canvas.inspector = inspectorOpen; throw failure; }
    });
    void tick().then(() => instructionsElement?.focus());
  }

  const redirectCandidates = $derived(selectedTask ? canvas.details.flatMap((task) => task.state === 'queued' && task.started_at === null ? Object.entries(task.definition.inputs).flatMap(([name, input]) => input.kind === 'task_output' && input.task === selectedTask.id ? [{ consumer: task.id, expected_version: task.version, input: name, label: task.definition.title }] : []) : []) : []);
  function corrects(task: TaskReadDto, original: string): boolean {
    const seen: string[] = [];
    let target = task.definition.corrective_of;
    while (target && !seen.includes(target)) { if (target === original) return true; seen.push(target); target = canvas.details.find((item) => item.id === target)?.definition.corrective_of ?? null; }
    return false;
  }
  function adoptCorrection() { if (!selectedTask || !correction) return; stage({ kind: 'adopt_correction', task: selectedTask.id, correction, redirects: redirectCandidates.filter((item) => redirectChoices.includes(`${item.consumer}:${item.input}`)).map(({ consumer, expected_version, input }) => ({ consumer, expected_version, input })) }); correctionOpen = false; }

  function supplyShared() {
    attempt(() => {
      if (!canvas.selected.startsWith('supplied:')) return;
      if (workflow && workflow.task_count > 200) throw new Error('This workflow exceeds the canvas’s 200-task review bound. Shared input replacement is unavailable here because every affected future task must be reviewed. Its current value and all drafts are intact.');
      if (workflow && canvas.details.length < workflow.task_count) throw new Error('Load the remaining task pages before replacing this shared input, so every affected future task can be shown.');
      if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation;
      stageChange(canvas, { kind: 'bind_input', name: canvas.selected.slice('supplied:'.length), value: { kind: 'text', text: supplyText } });
      canvas.notice = 'Input replacement added. Started tasks keep the input they already received.';
    });
  }

  function clearCache() {
    loadGeneration++; loading = false; canvas.details = []; canvas.error = '';
    const keep = [...canvas.tasks.map((task) => task.id), ...canvas.sources.map((source) => source.id)];
    canvas.positions = Object.fromEntries(Object.entries(canvas.positions).filter(([id]) => keep.includes(id)));
    canvas.notice = 'Cached details and their layout cleared. Select a task to reload it. Drafts and accepted connections are unchanged.';
  }

  async function downloadResult(task: TaskReadDto) { try { saveBlob(`${task.definition.title}-result.txt`, await client.download(`/v1/workflows/${encodeURIComponent(workflow!.id)}/tasks/${encodeURIComponent(task.id)}/result/content`)); } catch (failure) { report(failure); } }
  const submitLabel = $derived(canvas.target.kind === 'new' ? 'Start workflow' : canvas.tasks.length === 1 && canvas.tasks[0].version === null && !canvas.changes.length ? 'Add task' : 'Apply changes');
  const lanes = $derived(executionLanes(workflow?.tasks ?? [], workflow?.updated_at ?? Date.now()));
  function taskLabel(id: string) { const index = graph.nodes.filter((node) => node.kind === 'task').findIndex((node) => node.id === id); const title = graph.nodes.find((node) => node.id === id)?.label ?? canvas.details.find((task) => task.id === id)?.definition.title ?? 'Task outside this page'; return `${title}${graph.nodes.filter((node) => node.label === title).length > 1 ? ` · Task ${index + 1}` : ''}`; }
  function capabilityLabel(capability: TaskCapability) { const metadata = authoring(session, capability); const label = metadata?.label ?? capability.id; return `${label}${session.authoring.filter((item) => item.label === label).length > 1 ? ` · ${capability.id}` : ''}${metadata?.category === 'process' ? ' · Process' : ''}`; }
</script>

<svelte:window onkeydown={(event) => { if (event.key === 'Escape' && !event.defaultPrevented && event.target instanceof Node && inspector?.contains(event.target) && !(event.target instanceof HTMLSelectElement)) { event.preventDefault(); void closeInspector(); } }} />

<section class="canvas-workspace" aria-label="Workflow workspace">
  <header class="canvas-toolbar"><div class="canvas-title"><p class="eyebrow">{canvas.target.kind === 'new' ? 'Unsaved workflow' : session.daemon_name}</p><h1>{workflow?.title ?? (canvas.title || 'New workflow')}</h1></div><div class="canvas-toolbar-actions"><button class="quiet" onclick={() => { canvas.selected = 'workflow'; canvas.inspector = true; connection = null; edge = null; }}>Workflow</button><button class="primary" data-add-task onclick={addTask} disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>+ Add task</button><button onclick={() => addText()} disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>+ Text</button><button onclick={() => attachFile()} disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>Attach file</button></div></header>
  <input bind:this={attachment} class="visually-hidden" tabindex="-1" aria-label="Attach text file" type="file" accept="text/*,.md,.toml,.json,.csv,.rs,.txt" onchange={(event) => { const file = event.currentTarget.files?.[0]; if (file) void captureFile(file, attachmentTarget); event.currentTarget.value = ''; }} />
  <div class="canvas-body" class:inspector-visible={canvas.inspector}>
    {#if canvas.inspector}
      <aside class="canvas-inspector" aria-label="Selected item" bind:this={inspector}>
        <div class="inspector-top"><span class="eyebrow">{connection ? 'Connect' : edge ? 'Connection' : draftTask ? 'Unsaved task' : selectedTask ? 'Task' : selectedSource || selectedNode?.kind === 'source' ? 'Input' : 'Workflow'}</span><button class="quiet" aria-label="Close details" onclick={closeInspector}>×</button></div>
        {#if connection}
          <h2>{connection.order ? 'Run after…' : 'Connect to…'}</h2><p class="field-hint">From {taskLabel(connection.source)}</p>
          <label>Destination task<select value={connection.target} onchange={(event) => chooseDestination(event.currentTarget.value)}><option value="">Choose a task</option>{#each graph.nodes.filter((node) => node.kind === 'task' && node.id !== connection!.source) as node (node.id)}<option value={node.id}>{taskLabel(node.id)}</option>{/each}</select></label>
          {#if graph.nodes.find((node) => node.id === connection!.source)?.kind === 'task' && !connection.order}
            <label class="checkbox-label"><input type="checkbox" bind:checked={connection.outcome} /> Use the outcome even if it fails</label>
            {#if !connection.outcome}<label>Result to use<select bind:value={connection.output}><option value="">Choose a result</option>{#each connectionPorts as port (port.name)}<option value={port.name} disabled={'available' in port && !port.available}>{port.label}{port.required ? '' : ' · May be absent'}</option>{/each}</select></label>{#if connectionPorts.find((port) => port.name === connection!.output)?.required === false}<label class="checkbox-label"><input type="checkbox" bind:checked={connection.optional} /> Continue if this optional result is absent</label>{/if}<p class="field-hint">Waits for a successful result. A failed task blocks this input.</p>{:else}<p class="field-hint">Receives the retained outcome after the source settles, including a failure report.</p>{/if}
          {/if}
          {#if connection.target && !connection.order && availableSlots.length > 1 || connection.target && !connection.order && !targetContract?.allows_additional_inputs && availableSlots.length === 1}<label>Connect as<select bind:value={connection.slot}><option value="">Choose an input</option>{#each availableSlots as slot (slot.name)}<option value={slot.name}>{slot.label}</option>{/each}</select></label>{/if}
          {#if connection.target && !targetContract && !connection.order}<p class="notice warning">Choose a model or agent for the destination task before connecting inputs.</p>{/if}
          {#if outputPlan && Object.keys(outputPlan).length > 1}<p class="notice">This task shares its {(session.task_output_limit_bytes / 1048576).toLocaleString()} MiB result allowance across these connections. {Object.entries(outputPlan).map(([name, output]) => `${sourcePorts(connection!.source).find((port) => port.name === name)?.label ?? name}: ${(output.max_bytes / 1024).toLocaleString()} KiB`).join('; ')}. Connect applies these limits to the unsaved task.</p>{/if}
          <button class="primary" onclick={confirmConnection} disabled={!connection.target}>Connect</button>
        {:else if edge}
          <h2>{taskLabel(edge.source)} → {taskLabel(edge.target)}</h2><p>{edge.label}</p><p class="field-hint">{edge.draft ? 'This connection has not been submitted.' : 'Changing this connection affects only work that has not started.'}</p>
          {#if edge.binding?.kind === 'task_output'}<button onclick={() => attempt(() => { const task = editable(edge!.target); if (edge!.input) task.definition.inputs[edge!.input] = { kind: 'settled_outcome', task: edge!.source }; edge = null; })}>Use the outcome even if it fails</button>{/if}<button class="danger" onclick={deleteEdge}>Remove connection</button>
        {:else if draftTask}
          <form onsubmit={(event) => { event.preventDefault(); void submit(); }}>
            <fieldset disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>
              <label class="canvas-task-title">Task title<input value={draftTask.definition.title} placeholder="New task" maxlength="256" oninput={(event) => attempt(() => { updateDefinition(canvas, draftTask!, { ...draftTask!.definition, title: event.currentTarget.value }); draftTask!.titleEdited = true; })} /></label>
              <label>What should this task do?<textarea bind:this={instructionsElement} value={draftTask.definition.instructions} required maxlength="16384" rows="5" placeholder="Describe the work and what a useful result looks like." oninput={(event) => attempt(() => updateInstructions(canvas, draftTask!, event.currentTarget.value))}></textarea></label>
              <label class="model-assignment">Model or agent<select value={capabilityKey(draftTask.definition.capability)} onchange={(event) => attempt(() => { chooseCapability(draftTask!, session, event.currentTarget.value); pruneUnusedResults(draftTask!); oncapabilitychoice(draftTask!.definition.capability); })} required><option value="">Choose who does this task</option>{#each session.capabilities as capability (capabilityKey(capability))}<option value={capabilityKey(capability)}>{capabilityLabel(capability)}</option>{/each}</select></label>
              <button type="button" class="text-action" onclick={onmodels}>+ Add model or agent</button>
              {#if !session.capabilities.length}<p class="notice warning">Connect a model or agent before starting this task.</p>{:else if !contract && draftTask.definition.capability.id}<p class="notice warning">This selection’s input contract is unavailable. Refresh Models & agents; the draft is retained.</p>{/if}
              <div class="input-actions"><button type="button" onclick={() => addText(draftTask!.id)}>+ Add input</button><button type="button" class="quiet" onclick={() => attachFile(draftTask!.id)}>Attach file</button></div>
              {#if selectedEdges.length}<ul class="inspector-connections">{#each selectedEdges as input (input.id)}<li><button type="button" onclick={() => inspectEdge(input)}>{graph.nodes.find((node) => node.id === input.source)?.label}<small>{input.label}</small></button></li>{/each}</ul>{/if}
              <details><summary>More actions</summary><button type="button" onclick={() => addText(draftTask!.id, true)}>Wait for a required input</button><label>Run after<select value="" onchange={(event) => { if (event.currentTarget.value) attempt(() => { draftTask!.explicitDependencies = [...new Set([...draftTask!.explicitDependencies, event.currentTarget.value])]; synchronizeDependencies(draftTask!); }); }}><option value="">Choose a prerequisite task</option>{#each graph.nodes.filter((node) => node.kind === 'task' && node.id !== draftTask!.id) as node (node.id)}<option value={node.id}>{taskLabel(node.id)}</option>{/each}</select></label><label class="checkbox-label"><input type="checkbox" bind:checked={draftTask.definition.required} /> This task must succeed for the workflow to finish</label></details>
              {#if contract?.category === 'model'}<TaskPermissions definition={draftTask.definition} {session} onchange={(definition) => attempt(() => updateDefinition(canvas, draftTask!, definition))} onerror={(message) => canvas.error = message} />{/if}
              {#if draftTask.definition.corrective_of}<p class="field-hint">Corrects {taskLabel(draftTask.definition.corrective_of)}. The original result remains in history.</p>{/if}
            </fieldset>
            <div class="inspector-footer"><button class="primary" disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload || canvas.conflict}>{submitLabel}</button><button class="quiet" type="button" onclick={() => discardTask(draftTask!)} disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>Discard {draftTask.version === null ? 'task' : 'edit'}</button></div>
          </form>
        {:else if selectedTask}
          <h2>{selectedTask.definition.title}</h2><span class="badge {selectedTask.state}">{stateLabel(selectedTask)}</span>
          <p class="assigned-model">{authoring(session, selectedTask.definition.capability)?.label ?? selectedTask.definition.capability.id}</p>
          {#if selectedTask.reason}<p class="notice {selectedTask.state === 'failed' ? 'error' : 'warning'}">{readableReason(selectedTask.reason)}</p>{/if}
          {#if selectedTask.stop_requested && selectedTask.state !== 'stopped'}<p class="field-hint">Stop requested. The daemon has not reported that this task stopped.</p>{/if}
          <p class="task-instructions">{selectedTask.definition.instructions.slice(0, 700)}</p>
          {#if selectedTask.definition.instructions.length > 700}<details><summary>All instructions</summary><pre>{selectedTask.definition.instructions}</pre></details>{/if}
          {#if selectedTask.result}<h3>Result</h3><p class="result-preview">{selectedTask.result.text.slice(0, 600) || 'No displayable text was retained.'}</p><div class="input-actions"><button onclick={() => resultOpen = !resultOpen}>{resultOpen ? 'Hide result' : 'Read result'}</button><button onclick={() => beginConnection(selectedTask!.id)}>Use result</button></div>{#if resultOpen}<pre class="result-content" data-testid="result-output">{selectedTask.result.text}</pre>{#if selectedTask.result.text_truncated}<p class="field-hint">This is a bounded preview.</p>{/if}<button onclick={() => downloadResult(selectedTask!)}>Download result</button>{/if}
          {:else if selectedTask.result_restricted}<p class="notice warning">This connection cannot read the retained result.</p>{:else}<p class="field-hint">No result has been retained yet.</p>{/if}
          {#if selectedEdges.length}<h3>Inputs & prerequisites</h3><ul class="inspector-connections">{#each selectedEdges as input (input.id)}<li><button onclick={() => inspectEdge(input)}>{graph.nodes.find((node) => node.id === input.source)?.label}<small>{input.label}</small></button></li>{/each}</ul>{/if}
          <div class="inspector-actions">{#if selectedTask.state === 'queued' && selectedTask.started_at === null}<button onclick={() => attempt(() => editable(selectedTask!.id))}>Edit task</button><button class="danger" onclick={() => stage({ kind: 'remove', task: selectedTask!.id, expected_version: selectedTask!.version })}>Remove unstarted task</button>{/if}{#if ['queued', 'running', 'uncertain'].includes(selectedTask.state)}<button class="danger" disabled={selectedTask.stop_requested} onclick={() => stage({ kind: 'stop', task: selectedTask!.id })}>Request stop</button>{/if}<button onclick={() => createCorrection(selectedTask!)}>Create correction</button>{#if ['failed', 'stopped'].includes(selectedTask.state)}<button onclick={() => { correctionOpen = !correctionOpen; correction = ''; redirectChoices = []; }}>Use correction for future work</button>{/if}</div>
          {#if correctionOpen}<label>Successful correction<select bind:value={correction}><option value="">Choose completed corrective work</option>{#each canvas.details.filter((task) => task.state === 'completed' && corrects(task, selectedTask!.id)) as task (task.id)}<option value={task.id}>{taskLabel(task.id)}</option>{/each}</select></label><p class="field-hint">Select future tasks to receive the correction’s result. Started inputs remain unchanged.</p>{#each redirectCandidates as candidate (`${candidate.consumer}:${candidate.input}`)}<label class="checkbox-label"><input type="checkbox" value={`${candidate.consumer}:${candidate.input}`} bind:group={redirectChoices} /> {candidate.label}</label>{/each}{#if workflow && canvas.details.length < workflow.task_count}<p class="notice warning">Only loaded future consumers are listed. Load other pages to select additional consumers.</p>{/if}<button disabled={!correction} onclick={adoptCorrection}>Use selected correction</button>{/if}
          {#each workflow?.admissions.filter((admission) => admission.task_id === selectedTask!.id && admission.status === 'rejected') ?? [] as admission (admission.task_id)}<div class="notice warning"><h3>Follow-up proposal needs review</h3><p>{admission.reason}</p><p class="field-hint">Add any repaired tasks and connections to this drawing, then dismiss the proposal in the same submission. This does not resume a separate human wait.</p><button onclick={() => stage({ kind: 'dismiss_admission', task: admission.task_id })}>Dismiss reviewed proposal</button></div>{/each}
          <details><summary>Timing & history</summary><dl class="facts"><dt>Started</dt><dd>{selectedTask.started_at === null ? 'Not started' : timeLabel(selectedTask.started_at)}</dd><dt>Settled</dt><dd>{selectedTask.ended_at === null ? 'No settlement recorded' : timeLabel(selectedTask.ended_at)}</dd></dl>{#if selectedTask.definition.corrective_of}<p>Corrects {taskLabel(selectedTask.definition.corrective_of)}</p>{/if}{#if selectedTask.origin}<p>Created by a follow-up proposal from {taskLabel(selectedTask.origin.task_id)}.</p>{/if}</details>
          {#if selectedTask.frozen}<details><summary>Inputs used at start</summary>{#each Object.entries(selectedTask.frozen.inputs) as [name, input] (name)}<h3>{input.name ?? 'Captured input'}</h3>{#if input.restricted}<p>Access restricted.</p>{:else if input.text !== null}<pre>{input.text}</pre>{:else}<p>{input.kind === 'absent' ? 'Explicitly absent' : 'Captured immutable input'}</p>{/if}{#if input.text_truncated}<p class="field-hint">Bounded preview; the complete captured bytes are unchanged.</p>{/if}{/each}</details>{/if}
          <details><summary>Technical diagnostics</summary><dl class="facts"><dt>Task</dt><dd data-testid="task-id">{selectedTask.id}</dd><dt>Version</dt><dd>{selectedTask.version}</dd>{#if selectedTask.reason}<dt>Daemon reason</dt><dd>{selectedTask.reason}</dd>{/if}<dt>Result digest</dt><dd>{selectedTask.result?.digest ?? 'No result'}</dd></dl></details>
        {:else if selectedSource}
          <h2>{selectedSource.label}</h2>
          {#if selectedSource.value?.kind === 'uploaded'}<p class="field-hint">A captured file. This does not grant access to the daemon’s filesystem.</p>{:else}<label>Input text<textarea disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} value={selectedSource.text} rows="7" maxlength={session.input_limit_bytes} oninput={(event) => attempt(() => { updateSource(canvas, selectedSource!, event.currentTarget.value, session.input_limit_bytes); })}></textarea></label><label class="checkbox-label"><input type="checkbox" disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} checked={selectedSource.deferred} onchange={(event) => attempt(() => { updateSource(canvas, selectedSource!, selectedSource!.text, session.input_limit_bytes, event.currentTarget.checked); })} /> Supply this input later</label>{/if}
          <button onclick={() => beginConnection(selectedSource!.id)}>Connect to…</button>
          {#if sourceEdges.length}<h3>Used by</h3>{#each sourceEdges as item (item.id)}<button class="connection-row" onclick={() => select(item.target)}>{taskLabel(item.target)}</button>{/each}{:else}<p class="field-hint">Not connected yet. Connect it to the task that should receive it.</p><button class="quiet" disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => attempt(() => { assertEditable(canvas); const id = selectedSource!.id; canvas.sources = canvas.sources.filter((source) => source.id !== id); delete canvas.positions[id]; for (const key of Object.keys(canvas.bindings)) if (canvas.bindings[key] === id) delete canvas.bindings[key]; canvas.inspector = false; })}>Discard input</button>{/if}
        {:else if selectedNode?.kind === 'source'}
          <h2>{selectedNode.label}</h2><p>{selectedNode.detail}</p>
          {#if sourceEdges[0]}{@const item = sourceEdges[0]}{@const task = canvas.details.find((task) => task.id === item.target)}{@const binding = item.binding}{@const captured = item.input ? binding?.kind === 'supplied' ? (workflow?.supplied_inputs?.[binding.name] ?? task?.supplied_inputs?.[binding.name])?.value : task?.inline_inputs?.[item.input]?.value : null}{#if captured?.text}<pre>{captured.text}</pre>{:else if binding?.kind === 'inline' && binding.value.kind === 'text'}<pre>{binding.value.text}</pre>{/if}{/if}
          <h3>Used by</h3>{#each sourceEdges as item (item.id)}<button class="connection-row" onclick={() => select(item.target)}>{taskLabel(item.target)}</button>{/each}
          {#if canvas.selected.startsWith('supplied:')}
            <h3>Supply input</h3><p class="field-hint">Changes apply to future starts. Tasks that already started keep their captured input.</p>
            {@const suppliedName = canvas.selected.slice('supplied:'.length)}
            {@const consumers = canvas.details.filter((task) => task.started_at === null && task.state === 'queued' && Object.values(task.definition.inputs).some((input) => input.kind === 'supplied' && input.name === suppliedName))}
            <p class="field-hint">Affected future tasks: {consumers.map((task) => task.definition.title).join(', ') || 'None on loaded pages'}.</p>
            <label>Replacement text<textarea bind:value={supplyText} maxlength={session.input_limit_bytes} rows="4"></textarea></label><button onclick={supplyShared}>Supply input</button>
          {/if}
        {:else if selectedNode?.kind === 'boundary'}<h2>Task outside this page</h2><p class="field-hint">This connection points to retained work outside the loaded task page. It is not a missing task. Use the task page controls to inspect more work.</p>
        {:else if canvas.selected === 'workflow'}
          <h2>{workflow?.title ?? 'New workflow'}</h2>{#if !workflow}<label>Workflow name<input disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} bind:value={canvas.title} placeholder="Suggested from the first task" maxlength="256" /></label><label class="checkbox-label"><input type="checkbox" disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} bind:checked={canvas.finishWhenSettled} /> Finish when required work settles</label>{:else}<span class="badge {workflow.state}">{workflow.state}</span>{#if workflow.wait_question}<p class="notice warning">{workflow.wait_question}</p>{/if}<div class="inspector-actions"><button disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => pauseOpen = !pauseOpen}>Wait for a decision</button><button disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => stage({ kind: 'continue' })}>Resume workflow</button><button disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => stage({ kind: 'finish' })}>Finish when work settles</button><button disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => { limits = copy(workflow!.limits); limitsOpen = !limitsOpen; }}>Review limits</button><button onclick={onrefresh}>Refresh</button></div>{/if}
          {#if pauseOpen}<label>What needs a decision?<textarea bind:value={waitQuestion} rows="3" maxlength="4096"></textarea></label><button disabled={!waitQuestion.trim()} onclick={() => { stage({ kind: 'wait', question: waitQuestion }); pauseOpen = false; }}>Wait for this decision</button>{/if}
          {#if limitsOpen && limits}<p class="field-hint">These limits apply prospectively. Past spending and unresolved work remain accounted for.</p>{#each Object.entries(limitLabels) as [name, label] (name)}<label>{label}<input type="number" min="1" step="1" bind:value={limits[name as keyof WorkflowLimitsDto]} /></label>{/each}<button onclick={() => { if (limits) stage({ kind: 'set_limits', limits }); limitsOpen = false; }}>Use these limits</button>{/if}
          {#if workflow}<details><summary>Current limits & spending</summary><dl class="facts"><dt>Tasks</dt><dd>{workflow.admitted_tasks}/{workflow.limits.tasks}</dd><dt>Follow-up batches</dt><dd>{workflow.admitted_model_batches}/{workflow.limits.model_batches}</dd><dt>Rejected responses</dt><dd>{workflow.rejected_responses}/{workflow.limits.rejected_responses}</dd><dt>Execution time</dt><dd>{workflow.active_execution_ms}/{workflow.limits.active_execution_ms} ms</dd></dl></details>{/if}
          <p class="field-hint">The canvas holds up to 32 changes, 2 MiB of draft content and 200 loaded task details. Drafts live only in this authenticated browser session.</p>
          {#if canvas.details.length}<button class="quiet" onclick={clearCache}>Clear cached details</button>{/if}
        {:else}<p class="field-hint">{loading ? 'Loading task details…' : 'Select a card on the canvas.'}</p>{/if}
      </aside>
    {/if}
    <div class="canvas-view">
      <div class="canvas-view-tabs" aria-label="Task presentation"><button aria-pressed={canvas.presentation === 'canvas'} onclick={() => canvas.presentation = 'canvas'}>Canvas</button><button aria-pressed={canvas.presentation === 'list'} onclick={() => canvas.presentation = 'list'}>List</button><button aria-pressed={canvas.presentation === 'timeline'} onclick={() => canvas.presentation = 'timeline'}>Timeline</button></div>
      {#if canvas.presentation === 'canvas'}<WorkflowCanvas {canvas} nodes={graph.nodes} edges={graph.edges} onselect={select} onconnect={beginConnection} ondropfile={(file, target) => captureFile(file, target)} onedge={inspectEdge} />
      {:else if canvas.presentation === 'list'}<div class="canvas-list" role="region" aria-label="Tasks and inputs">{#each graph.nodes as node (node.id)}<article><button class="list-node" aria-pressed={canvas.selected === node.id} onclick={() => select(node.id)}><strong>{node.label}</strong><span class="badge {node.state}">{node.draft ? 'Unsaved' : node.state}</span><p>{node.detail}</p></button>{#if node.kind !== 'boundary'}<button class="quiet" onclick={() => beginConnection(node.id)}>Connect to…</button>{/if}</article>{/each}<h2>Connections</h2>{#each graph.edges as item (item.id)}<button onclick={() => inspectEdge(item)}>{taskLabel(item.source)} → {taskLabel(item.target)} · {item.label}</button>{/each}</div>
      {:else}<div class="canvas-timeline" role="region" aria-label="Actual task execution"><p class="field-hint">Actual execution times. Queued tasks have no start time.</p>{#each lanes as lane (lane.task.id)}<div class="execution-row"><button class="quiet execution-label" onclick={() => select(lane.task.id)}>{lane.task.title}</button><div class="execution-track"><button class="execution-bar" class:open={lane.task.ended_at === null} style={`left:${lane.left}%;width:${Math.max(.5, lane.width)}%`} aria-label={`${lane.task.title}: started ${timeLabel(lane.task.started_at)}, ${lane.task.ended_at === null ? 'no end recorded' : `ended ${timeLabel(lane.task.ended_at)}`}`} onclick={() => select(lane.task.id)}></button></div></div>{:else}<p>No tasks have a recorded start yet.</p>{/each}</div>{/if}
      {#if !graph.nodes.length}<div class="canvas-empty"><span aria-hidden="true">✦</span><h2>{workflow ? 'No tasks on this page' : 'Give your first task some space'}</h2><p>Describe the work. Connect inputs when you need them.</p><button class="primary" onclick={addTask}>Add a task</button></div>{/if}
      {#if graph.truncated}<p class="graph-boundary">This drawing exceeds the 600-card or 1,024-connection display bound. Refine the loaded page before editing; undisplayed connections are retained.</p>{/if}
      {#if workflow && workflow.task_count > workflow.tasks.length}<div class="graph-boundary"><span>Showing {workflow.tasks.length} of {workflow.task_count} tasks. Off-page connections stay intact.</span><button onclick={() => onpage()}>First task page</button>{#if workflow.next_task_cursor}<button onclick={() => onpage(workflow!.next_task_cursor!)}>Next task page</button>{/if}</div>{/if}
    </div>
  </div>
  <footer class="canvas-status">
    <div aria-live="polite">{#if canvas.notice}<span>{canvas.notice}</span>{:else if workflow}<span class="status-dot" aria-hidden="true"></span><span>{workflow.state} · {workflow.task_count} tasks</span>{:else}<span>Unsaved · Nothing runs until you start the workflow</span>{/if}</div>
    {#if hasChanges(canvas) && !canvas.pending}<div class="drawing-actions"><span>{changeCount(canvas)} unsaved {changeCount(canvas) === 1 ? 'change' : 'changes'}</span><button class="quiet" disabled={canvas.busy || !!canvas.pendingUpload} onclick={ondiscard}>Discard drawing</button><button class="primary" disabled={canvas.busy || canvas.conflict || !!canvas.pendingUpload || !changeCount(canvas)} onclick={() => submit()}>{canvas.busy ? 'Submitting…' : submitLabel}</button></div>{/if}
  </footer>
  {#if canvas.changes.length}<details class="drawing-review"><summary>Changes to apply ({changeCount(canvas)})</summary><ul>{#each canvas.tasks as task (task.id)}<li>{task.version === null ? 'Add' : 'Edit'} {task.definition.title || 'New task'} · {Object.keys(task.definition.inputs).length} inputs</li>{/each}{#each canvas.changes as change, index (index)}<li>{change.kind === 'bind_input' ? 'Supply input for future tasks' : change.kind === 'adopt_correction' ? `Use ${taskLabel(change.correction)} for ${taskLabel(change.task)}` : change.kind.replaceAll('_', ' ')}{'task' in change && change.kind !== 'adopt_correction' ? `: ${taskLabel(change.task)}` : ''}<button class="quiet" disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => canvas.changes = canvas.changes.filter((_, at) => at !== index)}>Undo</button></li>{/each}</ul></details>{/if}
  {#if canvas.error}<p class="notice error canvas-message" role="alert">{canvas.error}</p>{/if}
  {#if canvas.conflict}<div class="notice warning canvas-message">The workflow changed. Your drawing is intact. Review the current tasks before another submission. <button disabled={canvas.busy} onclick={refreshConflict}>Refresh workflow for review</button></div>{/if}
  {#if canvas.pending}<div class="notice warning canvas-message"><p>The reply is unconfirmed. Its exact destination and drawing are retained.</p><button disabled={canvas.busy} onclick={() => submit(true)}>Find submission receipt</button><button disabled={canvas.busy} onclick={() => submit()}>Replay exact submission</button></div>{/if}
  {#if canvas.pendingUpload}<div class="notice warning canvas-message"><p>The file capture reply is unconfirmed.</p><button disabled={canvas.busy} onclick={() => captureFile()}>Retry exact upload</button></div>{/if}
</section>
