<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import type { WorkflowTopologyDto, AuthoringSnapshotDto, AuthoringDocumentDto, PositionsSnapshotDto, AuthoringReviewDto, ReviewAuthoringRequest, RunAuthoringRequest, ProjectDto, TaskCapability, TaskChangeDto, TaskReadDto, TaskSessionDto, TaskUploadReceipt, WorkflowLimitsDto, WorkflowReadDto, WorkflowReceipt } from '../lib/protocol.generated';
  import { addCanvasTask, addSource, addConnection, resolveConnections, unusedCanvasReferences, removeUnusedCanvasReferences, useUsualModel, canvasDocument, adoptAuthoring, copyTaskConnections, assertCapacity, assertEditable, authoring, cacheTask, capabilityKey, changeCount, chooseCapability, copy, hasChanges, projectGraph, reviseTask, stageChange, updateDefinition, updateInstructions, updateSource, type CanvasState, type CanvasTask, type GraphEdge } from '../lib/canvas';
  import { ConditionalCheckpoint } from '../lib/checkpoint';
  import { DaemonClient, DaemonError } from '../lib/transport';
  import { captureTextRequest } from '../lib/task-input';
  import { executionLanes, readableReason, saveBlob, stateLabel, timeLabel } from '../lib/presentation';
  import WorkflowCanvas from './WorkflowCanvas.svelte';
  import TaskPermissions from './TaskPermissions.svelte';
  import ExecutionDetails from './ExecutionDetails.svelte';
  import '../canvas.css';

  let { canvas = $bindable(), currentProject = null, workflow, session, client, onaccepted, onreviewed, onrefresh, onpage, onerror, onmodels }: {
    canvas: CanvasState; workflow: WorkflowReadDto | null; session: TaskSessionDto; client: DaemonClient;
    currentProject?: ProjectDto | null;
    onaccepted: (receipt: WorkflowReceipt) => Promise<void>;
    onreviewed: (workflow: WorkflowReadDto) => void; onrefresh: () => Promise<void>;
    onpage: (cursor?: string) => Promise<void>; onerror: (failure: unknown) => void;
    onmodels: () => void;
  } = $props();
  let graph = $derived.by(() => {
    const projection = projectGraph(canvas, workflow);
    for (const node of projection.nodes) {
      const task = canvas.tasks.find((item) => item.id === node.id) ?? canvas.details.find((item) => item.id === node.id);
      const topology = canvas.topology?.tasks.find((item) => item.id === node.id);
      const metadata = authoring(session, task?.definition.capability ?? topology?.capability ?? { id: '', operation: '' });
      if (node.kind === 'task') {
        node.model = metadata?.label ?? (task?.definition.capability.id ? 'Model unavailable' : 'Choose a model');
        node.inputs = metadata?.inputs.length ? metadata.inputs.map(({ name, label }) => ({ name, label })) : [{ name: null, label: 'Input' }];
        node.outputs = metadata?.outputs.length ? metadata.outputs.map(({ name, label }) => ({ name, label })) : [{ name: null, label: 'Output' }];
      }
    }
    return projection;
  });
  let draftTask = $derived(canvas.tasks.find((task) => task.id === canvas.selected));
  let selectedTask = $derived(canvas.details.find((task) => task.id === canvas.selected));
  let selectedSource = $derived(canvas.sources.find((source) => source.id === canvas.selected));
  let selectedNode = $derived(graph.nodes.find((node) => node.id === canvas.selected));
  let selectedEdges = $derived(graph.edges.filter((edge) => edge.target === canvas.selected));
  let sourceEdges = $derived(graph.edges.filter((edge) => edge.source === canvas.selected));
  let contract = $derived(draftTask ? authoring(session, draftTask.definition.capability) : selectedTask ? authoring(session, selectedTask.definition.capability) : undefined);
  let usualChoice = $state('');
  let titleEditing = $state(false);
  let titleDraft = $state('');
  let usualOpen = $state(false);
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
  let correctionsLoading = $state(false);
  let correctionError = $state('');
  let correctionDiscovery = 0;
  let resultOpen = $state(false);
  let loading = $state(false);
  let instructionsElement: HTMLTextAreaElement | undefined = $state();
  let attachment: HTMLInputElement;
  let attachmentTarget = '';
  let inspector: HTMLElement | undefined = $state();
  let mounted = true;
  let ready = $state(false);
  let scopeOpen = $state(false);
  let scope = $state<string[] | null>(null);
  let useCurrent = $state<string[]>([]);
  let review = $state<AuthoringReviewDto | null>(null);
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let positionTimer: ReturnType<typeof setTimeout> | undefined;
  const workflowPath = untrack(() => canvas.target.kind === 'existing' ? `/v1/workflows/${encodeURIComponent(canvas.target.workflowId)}` : '');
  const checkpoint = untrack(() => canvas.checkpoint ??= new ConditionalCheckpoint<AuthoringDocumentDto>(canvas.authoringSave,
    () => client.request<AuthoringSnapshotDto>(`${workflowPath}/authoring`),
    (expected_generation, document) => client.put<AuthoringSnapshotDto>(`${workflowPath}/authoring`, { expected_generation, document }), onerror));
  const positions = untrack(() => canvas.positionCheckpoint ??= new ConditionalCheckpoint<Record<string, { x: number; y: number }>>(canvas.positionSave,
    async () => { const value = await client.request<PositionsSnapshotDto>(`${workflowPath}/positions`); return { ...value, document: value.positions }; },
    async (expected_generation, value) => { const saved = await client.put<PositionsSnapshotDto>(`${workflowPath}/positions`, { expected_generation, positions: value }); return { ...saved, document: saved.positions }; }, onerror));
  async function loadAuthoring() {
    const current = canvas;
    try {
      if (current.authoringSave.status === 'loading' || current.authoringSave.status === 'saved') { const document = await checkpoint.load(); if (current.disposed) return; adoptAuthoring(current, document); }
      if (current.positionSave.status === 'loading' || current.positionSave.status === 'saved') { const saved = await positions.load(); if (current.disposed) return; current.positions = saved; }
      if (mounted && !current.disposed) ready = true;
    } catch (failure) { if (mounted && !current.disposed) report(failure); }
  }
  $effect(() => { if (workflowPath) untrack(() => void loadAuthoring()); });
  $effect(() => {
    const document = canvasDocument(canvas), loaded = ready;
    if (loaded) untrack(() => { checkpoint.stage(document); clearTimeout(saveTimer); saveTimer = setTimeout(() => void checkpoint.flush(), 350); review = null; });
  });
  $effect(() => {
    const current = copy(canvas.positions), loaded = ready;
    if (loaded) untrack(() => { positions.stage(current); clearTimeout(positionTimer); positionTimer = setTimeout(() => void positions.flush(), 250); });
  });
  let loadGeneration = 0;
  let topologyCursor = $state<string | null>(null);
  const limitLabels: Record<keyof WorkflowLimitsDto, string> = { tasks: 'Total tasks', concurrent_tasks: 'Tasks at once', model_batches: 'Follow-up batches', rejected_responses: 'Consecutive refused proposals', corrective_failures: 'Failed corrections in a row', active_execution_ms: 'Execution time (milliseconds)', tool_calls_per_task: 'Tool calls per task' };
  onDestroy(() => { mounted = false; loadGeneration++; clearTimeout(saveTimer); clearTimeout(positionTimer); void checkpoint.flush(); void positions.flush(); });

  $effect(() => {
    const snapshot = workflow;
    const signature = snapshot?.tasks.map((task) => `${task.id}:${task.version}:${task.state}:${task.entered}:${task.has_result}:${task.stop_requested}:${task.reason}:${task.started_at}:${task.ended_at}`).join('|');
    if (snapshot && signature) untrack(() => void loadPage(snapshot));
  });
  $effect(() => {
    const initial = graph.nodes.filter((node) => !canvas.positions[node.id]).map((node) => ({ id: node.id, position: node.position }));
    untrack(() => { for (const node of initial) if (Object.keys(canvas.positions).length < 600) canvas.positions[node.id] = node.position; });
  });

  async function loadPage(snapshot: WorkflowReadDto, cursor: string | null = topologyCursor) {
    const current = canvas;
    const generation = ++loadGeneration;
    loading = true;
    try {
      const topology = await client.request<WorkflowTopologyDto>(`/v1/workflows/${encodeURIComponent(snapshot.id)}/topology${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ''}`);
      if (!mounted || current.disposed || generation !== loadGeneration) return;
      current.topology = topology; topologyCursor = cursor;
      const selected = snapshot.tasks.find((task) => task.id === current.selected);
      const cached = current.details.find((task) => task.id === current.selected);
      if (selected && (!cached || cached.version !== selected.version || cached.state !== selected.state || cached.reason !== selected.reason || cached.stop_requested !== selected.stop_requested || (cached.frozen?.entered === true) !== selected.entered || Boolean(cached.result || cached.result_restricted) !== selected.has_result)) {
        const detail = await client.request<TaskReadDto>(`/v1/workflows/${encodeURIComponent(snapshot.id)}/tasks/${encodeURIComponent(selected.id)}`);
        if (mounted && !current.disposed && generation === loadGeneration) cacheTask(current, detail);
      }
    } catch (failure) {
      if (mounted && !current.disposed && generation === loadGeneration) {
        if (cursor && failure instanceof DaemonError && failure.status === 409) { topologyCursor = null; await loadPage(snapshot, null); }
        else report(failure);
      }
    } finally { if (generation === loadGeneration) loading = false; }
  }

  async function loadTask(id: string) {
    const owner = canvas;
    const current = owner.details.find((task) => task.id === id), summary = workflow?.tasks.find((task) => task.id === id);
    if (current && (!summary || current.version === summary.version && current.state === summary.state && current.reason === summary.reason)) return current;
    const task = await client.request<TaskReadDto>(`${workflowPath}/tasks/${encodeURIComponent(id)}`);
    if (owner.disposed) throw new Error('This connection is no longer available.');
    cacheTask(owner, task); return task;
  }
  async function materializeSource(id: string) {
    if (canvas.sources.some((item) => item.id === id) || graph.nodes.find((item) => item.id === id)?.kind !== 'source') return;
    const existing = graph.edges.find((item) => item.source === id && item.input);
    if (!existing?.input) throw new Error('Load this input’s originating task before connecting it.');
    const task = await loadTask(existing.target);
    if (!mounted) return;
    const binding = task.definition.inputs[existing.input];
    const observed = binding?.kind === 'supplied' ? task.supplied_inputs[binding.name] : task.inline_inputs[existing.input];
    if (observed?.availability === 'restricted' || observed?.availability === 'expired' || observed?.value?.restricted) throw new Error('This input is restricted or expired. Choose an available source.');
    if (!binding || binding.kind !== 'inline' && binding.kind !== 'supplied') throw new Error('This source changed. Refresh its connections.');
    canvas.sources.push({ id, label: graph.nodes.find((item) => item.id === id)?.label ?? 'Task input', value: binding.kind === 'inline' ? copy(binding.value) : null, sharedName: binding.kind === 'supplied' ? binding.name : null, text: binding.kind === 'inline' && binding.value.kind === 'text' ? binding.value.text : '', deferred: binding.kind === 'supplied' });
  }

  function report(failure: unknown) { if (canvas && !canvas.disposed) canvas.error = failure instanceof Error ? failure.message : String(failure); onerror(failure); }
  function attempt(action: () => void) { try { if (!ready) throw new Error('Wait for the saved canvas to load.'); canvas.error = ''; canvas.notice = ''; action(); } catch (failure) { report(failure); } }
  function editable(id: string): CanvasTask {
    assertEditable(canvas);
    const draft = canvas.tasks.find((item) => item.id === id);
    if (draft) return draft;
    const task = canvas.details.find((item) => item.id === id);
    if (!task) throw new Error('Load this task’s details before changing it. Tasks outside this page remain intact.');
    if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation;
    return reviseTask(canvas, task, session);
  }

  export async function addTask(point?: { x: number; y: number }) {
    attempt(() => {
      if (!ready) throw new Error('Wait for the saved canvas to load.');
      if (!canvas.target.projectId) throw new Error('Choose an accessible project before adding a task. Earlier workflows can be associated from their project notice.');
      if (!hasChanges(canvas) && currentProject) canvas.project = copy(currentProject);
      if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation;
      const task = addCanvasTask(canvas, session);
      if (point) canvas.positions[task.id] = point;
      else {
        const surface = document.querySelector<HTMLElement>('.graph-surface');
        if (surface) {
          const origin = { x: (Math.max(260, surface.clientWidth - (canvas.inspector ? 330 : 0)) / 2 - canvas.camera.x) / canvas.camera.zoom - 115, y: (surface.clientHeight / 2 - canvas.camera.y) / canvas.camera.zoom - 65 };
          const occupied = Object.entries(canvas.positions).filter(([id]) => id !== task.id).map(([, value]) => value);
          for (let index = 0; index <= occupied.length; index++) {
            const proposed = { x: Math.max(-2000, Math.min(12000, origin.x + (index % 3) * 290)), y: Math.max(-2000, Math.min(12000, origin.y + Math.floor(index / 3) * 205)) };
            if (!occupied.some((value) => Math.abs(value.x - proposed.x) < 250 && Math.abs(value.y - proposed.y) < 175)) { canvas.positions[task.id] = proposed; break; }
          }
        }
      }
    });
  }
  export function canNavigate() { return true; }

  function select(id: string) {
    correctionDiscovery++; correctionsLoading = false; correctionError = '';
    canvas.selected = id; canvas.inspector = true; connection = null; edge = null; correctionOpen = false; resultOpen = false; supplyText = '';
    if (workflow && (workflow.tasks.some((task) => task.id === id) || canvas.topology?.tasks.some((task) => task.id === id) || graph.nodes.find((item) => item.id === id)?.kind === 'boundary') && !canvas.details.some((task) => task.id === id)) {
      const snapshot = workflow;
      void client.request<TaskReadDto>(`/v1/workflows/${encodeURIComponent(snapshot.id)}/tasks/${encodeURIComponent(id)}`).then((task) => { if (mounted && !canvas.disposed) cacheTask(canvas, task); }).catch((failure) => { if (mounted && !canvas.disposed) report(failure); });
    }
  }
  async function closeInspector() { const current = canvas; current.inspector = false; connection = null; edge = null; await tick(); if (mounted && !current.disposed) document.querySelector<HTMLButtonElement>(`[data-node="${CSS.escape(current.selected)}"] .node-content`)?.focus(); }

  function sourcePorts(id: string) {
    const draft = canvas.tasks.find((task) => task.id === id);
    const actual = canvas.details.find((task) => task.id === id);
    const topology = canvas.topology?.tasks.find((task) => task.id === id);
    const metadata = authoring(session, draft?.definition.capability ?? actual?.definition.capability ?? topology?.capability ?? { id: '', operation: '' });
    if (draft?.version === null) return metadata?.outputs ?? [];
    const declarations = actual?.definition.outputs ?? topology?.outputs ?? {};
    const published = actual?.result?.outputs ?? topology?.published_outputs ?? {};
    return [...new Set([...Object.keys(declarations), ...Object.keys(published)])].map((name) => ({ name, label: `${metadata?.outputs.find((port) => port.name === name)?.label ?? name.replaceAll('_', ' ')}${actual?.result && !published[name] ? ' · Not published' : ''}`, required: declarations[name]?.required ?? true, media_type: declarations[name]?.media_type ?? published[name]?.media_type ?? null, max_bytes: declarations[name]?.max_bytes ?? session.task_output_limit_bytes, available: actual?.result_restricted ? false : published[name]?.available ?? (declarations[name]?.required === false || !actual?.result) }));
  }

  const connectionPorts = $derived(connection ? sourcePorts(connection.source) : []);
  const targetDraft = $derived(connection ? canvas.tasks.find((task) => task.id === connection!.target) : undefined);
  const targetContract = $derived.by(() => {
    const capability = targetDraft?.definition.capability ?? canvas.topology?.tasks.find((task) => task.id === connection?.target)?.capability;
    return capability ? authoring(session, capability) : undefined;
  });
  const availableSlots = $derived(targetContract?.inputs.filter((slot) => !(slot.name in (targetDraft?.definition.inputs ?? {}))) ?? []);
  function beginConnection(source: string, target = '', order = false) {
    attempt(() => {
      if (canvas.pending || canvas.pendingUpload || canvas.busy) throw new Error('Recover this pending submission before changing connections.');
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
      connection.target = id;
      const capability = canvas.tasks.find((task) => task.id === id)?.definition.capability ?? canvas.topology?.tasks.find((task) => task.id === id)?.capability;
      const metadata = capability ? authoring(session, capability) : undefined;
      connection.slot = metadata?.inputs.length === 1 && !metadata.allows_additional_inputs ? metadata.inputs[0].name : '';
    });
  }

  async function directConnection(source: string, target = '', output: string | null = null, input: string | null = null) {
    if (!target) { beginConnection(source); return; }
    try {
      await materializeSource(source);
      if (!mounted) return;
      if (!canvas.tasks.some((item) => item.id === target)) await loadTask(target);
      if (!mounted) return;
      editable(target);
      const wire = addConnection(canvas, session, source, target, output, input);
      canvas.notice = wire.input && (wire.output || canvas.sources.some((item) => item.id === source)) ? 'Connected.' : 'Wire placed. Choose its missing endpoint when you are ready.';
    } catch (failure) { report(failure); }
  }

  async function confirmConnection() {
    const pending = connection, owner = canvas;
    if (!pending) return;
    const target = pending.target;
    try {
      await materializeSource(pending.source);
      if (!mounted || owner.disposed || connection !== pending || connection.target !== target) return;
      if (target && !owner.tasks.some((item) => item.id === target)) await loadTask(target);
      if (!mounted || owner.disposed || connection !== pending || connection.target !== target) return;
      attempt(() => {
      if (!connection?.source || !connection.target) throw new Error('Choose the receiving task.');
      editable(connection.target);
      const wire = addConnection(canvas, session, connection.source, connection.target, connection.output || null, connection.slot || null, connection.order ? 'after_success' : 'data');
      wire.outcome = connection.outcome; wire.optional = connection.optional;
      connection = null; canvas.notice = 'Connection placed.';
      });
    } catch (failure) { if (mounted && !owner.disposed && connection === pending && connection.target === target) report(failure); }
  }

  function addText(target = '', missing = false) {
    attempt(() => {
      if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation;
      const source = addSource(canvas, missing ? 'Required input' : 'Text input', missing ? null : { kind: 'text', text: '' });
      if (target) {
        editable(target); addConnection(canvas, session, source.id, target);
      }
    });
  }

  async function captureFile(file?: File, target = '') {
    const current = canvas, connectionClient = client, reportError = onerror;
    if (!ready || current.busy || current.disposed || current.pending || file && current.pendingUpload) return;
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
        if (!current.tasks.some((task) => task.id === pending.target)) {
          const task = current.details.find((task) => task.id === pending.target);
          if (!task) throw new Error('The captured file is saved as an input. Load the receiving task before connecting it.');
          reviseTask(current, task, session);
        }
        addConnection(current, session, source.id, pending.target);
      }
    } catch (failure) {
      if (current.disposed) return;
      current.error = failure instanceof Error ? failure.message : 'The file capture could not be confirmed.';
      if (failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) current.pendingUpload = null;
      reportError(failure);
    } finally { if (!current.disposed) current.busy = false; }
  }

  function attachFile(target = '') { attachmentTarget = target; attachment.click(); }

  async function reviewScope(runWhenUnchanged = false) {
    const current = canvas;
    if (current.busy || current.pending || !ready) return;
    current.error = ''; current.busy = true;
    try {
      checkpoint.stage(canvasDocument(current)); await checkpoint.flush();
      if (current.authoringSave.status !== 'saved') throw new Error('Save this canvas before running its plan. Your edits are retained.');
      const request: ReviewAuthoringRequest = { authoring_generation: current.authoringSave.generation, tasks: scope ?? current.tasks.map((task) => task.id), changes: current.changes.map((_, index) => index), use_current: useCurrent };
      const result = await client.request<AuthoringReviewDto>(`${workflowPath}/authoring/review`, request);
      if (current.disposed) return;
      review = result;
      const additional = result.tasks.filter((id) => !request.tasks.includes(id));
      if (runWhenUnchanged && !result.issues.length && !additional.length) { current.busy = false; await runReviewed(result, current); }
      else { scope = result.tasks; scopeOpen = true; current.highlighted = additional; }
    } catch (failure) { if (!current.disposed) report(failure); }
    finally { if (!current.disposed) current.busy = false; }
  }
  async function runReviewed(value: AuthoringReviewDto, current: CanvasState = canvas) {
    if (current.busy || current.pending || current.disposed || value.issues.length) return;
    const request: RunAuthoringRequest = { request_id: crypto.randomUUID(), expected_generation: value.expected_generation, authoring_generation: value.authoring_generation, tasks: value.tasks, changes: value.changes, use_current: value.use_current };
    current.pending = { path: `${workflowPath}/authoring/run`, request, run: true };
    await submit(false, current);
  }

  async function submit(receiptOnly = false, current: CanvasState = canvas) {
    const connectionClient = client, accept = onaccepted, refresh = onrefresh, reportError = onerror;
    if (current.disposed || current.busy || current.pendingUpload || current.conflict && !current.pending?.control) return;
    const pending = current.pending;
    const control = pending?.control;
    current.error = ''; current.busy = true;
    try {
      if (!pending) return;
      const receipt = await connectionClient.request<WorkflowReceipt>(receiptOnly ? `/v1/workflow-requests/${encodeURIComponent(pending.request.request_id)}` : pending.path, receiptOnly ? undefined : pending.request);
      if (current.disposed) return;
      for (const [alias, id] of Object.entries(receipt.aliases)) if (current.positions[alias] && alias !== id) { current.positions[id] = current.positions[alias]; delete current.positions[alias]; }
      if (pending.control && 'changes' in pending.request && !('authoring_generation' in pending.request)) {
        current.pending = null;
        if (current.target.kind === 'existing' && current.target.generation === pending.request.expected_generation) current.target.generation = receipt.generation;
        const change = pending.request.changes[0];
        current.notice = change.kind === 'wait' ? 'Pause accepted. Work that has already entered may continue.'
          : change.kind === 'continue' ? 'Resume accepted. Eligible work can continue.'
          : change.kind === 'rename' ? 'Workflow renamed.' : 'Stop requested. Waiting for the daemon to report the outcome.';
        if (mounted) await refresh();
      } else if (pending.run) {
        const selected = current.selected;
        const document = await checkpoint.load();
        if (current.disposed) return;
        adoptAuthoring(current, document); current.pending = null;
        current.selected = receipt.aliases[selected] ?? selected;
        scope = null; useCurrent = []; review = null; scopeOpen = false;
        await accept(receipt);
      } else { current.pending = null; await accept(receipt); }
    } catch (failure) {
      if (current.disposed) return;
      current.error = failure instanceof Error ? failure.message : 'The daemon reply could not be confirmed.';
      if (!receiptOnly && failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) {
        current.pending = null;
        if (!control) {
          current.conflict = current.target.kind === 'existing' && failure.status === 409;
          if (pending?.run && 'authoring_generation' in pending.request && typeof failure.details === 'object' && failure.details !== null && 'authoring_issues' in failure.details) {
            try {
              const issues = JSON.parse(String(failure.details.authoring_issues));
              if (Array.isArray(issues) && issues.every((issue) => issue && typeof issue.code === 'string' && typeof issue.message === 'string')) {
                review = { ...pending.request, issues }; scope = [...pending.request.tasks]; scopeOpen = true;
              }
            } catch { /* A malformed detail cannot replace the daemon's primary refusal. */ }
          }
        }
        else if (failure.status === 409) {
          current.notice = 'The workflow changed. Review its current state before choosing this action again. Your drawing is retained.';
          if (mounted) await refresh();
        }
      }
      reportError(failure);
      if (!receiptOnly && current.pending?.run && !(failure instanceof DaemonError && failure.status < 500)) {
        current.busy = false; await submit(true, current);
      }
    } finally { if (!current.disposed) current.busy = false; }
  }

  async function control(change: Extract<TaskChangeDto, { kind: 'wait' | 'continue' | 'stop' | 'rename' }>) {
    if (!workflow || canvas.target.kind !== 'existing' || canvas.disposed || canvas.busy || canvas.pending || canvas.pendingUpload) return;
    canvas.pending = {
      path: `/v1/workflows/${encodeURIComponent(workflow.id)}/changes`, control: true,
      request: { request_id: change.kind === 'stop' ? canvas.details.find((task) => task.id === change.task)!.stop_request_id : crypto.randomUUID(), expected_generation: workflow.generation, changes: [change] },
    };
    await submit();
  }

  async function renameWorkflow() {
    if (!titleEditing || !workflow) return;
    const title = titleDraft.trim();
    if (!title || title === workflow.title) { titleEditing = false; return; }
    titleEditing = false;
    await control({ kind: 'rename', title, description: workflow.description });
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
      if (task.version === null && canvas.connections.some((item) => item.source === task.id)) throw new Error('Disconnect this draft’s consumers before discarding it.');
      canvas.tasks = canvas.tasks.filter((item) => item.id !== task.id);
      canvas.connections = canvas.connections.filter((item) => item.target !== task.id);
      if (task.version === null) delete canvas.positions[task.id];
      canvas.inspector = false; canvas.selected = ''; canvas.conflict = false;
    });
  }

  function inspectEdge(value: GraphEdge) { edge = value; connection = null; canvas.inspector = true; }
  function deleteEdge() {
    attempt(() => {
      if (!edge) return;
      if (!canvas.tasks.some((item) => item.id === edge!.target)) editable(edge.target);
      canvas.connections = canvas.connections.filter((item) => item.id !== edge!.id);
      edge = null; canvas.notice = 'Connection removed. Other data and independent ordering wires remain.';
    });
  }

  function createCorrection(task: TaskReadDto) {
    attempt(() => {
      if (!canvas.target.projectId) throw new Error('Choose an accessible project before creating a correction.');
      if (task.definition.execution?.kind === 'restricted') throw new Error('This task’s execution selection is restricted. Create a new task with an accessible project and reviewed permissions.');
      if (!hasChanges(canvas) && currentProject) canvas.project = copy(currentProject);
      if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation;
      const selected = canvas.selected, inspectorOpen = canvas.inspector;
      const draft = addCanvasTask(canvas, session, task.definition.capability);
      try {
      const original = copy(task.definition), binding = original.execution?.kind === 'bound' ? original.execution.binding : null;
      draft.definition = { ...original, title: `Correction: ${task.definition.title}`, instructions: '', corrective_of: task.id, execution: binding && canvas.project ? { kind: 'explicit', expected_generation: canvas.project.generation, settings: binding.settings, access: binding.access, tools: binding.tools } : draft.definition.execution, paths: binding?.paths ?? [], tools: [] };
      draft.titleEdited = true; copyTaskConnections(canvas, task, draft.id); draft.definition.inputs = {}; draft.definition.dependencies = {};
      assertCapacity(canvas);
      canvas.selected = draft.id; canvas.inspector = true;
      if (binding) canvas.notice = 'This correction keeps the original environment and permissions. The daemon checks them against current authority when you submit it.';
      } catch (failure) { canvas.tasks = canvas.tasks.filter((item) => item.id !== draft.id); delete canvas.positions[draft.id]; canvas.selected = selected; canvas.inspector = inspectorOpen; throw failure; }
    });
    void tick().then(() => instructionsElement?.focus());
  }

  const redirectCandidates = $derived(selectedTask ? canvas.details.flatMap((task) => task.state === 'queued' && task.started_at === null ? Object.entries(task.definition.inputs).flatMap(([name, input]) => input.kind === 'task_output' && input.task === selectedTask.id ? [{ consumer: task.id, expected_version: task.version, input: name, label: task.definition.title }] : []) : []) : []);
  async function discoverCorrections() {
    const owner = canvas, selected = owner.selected, discovery = ++correctionDiscovery;
    correctionOpen = true; correction = ''; redirectChoices = []; correctionError = ''; correctionsLoading = true;
    // Correction relationships are only available in authorized task reads. Fetch this
    // bounded page deliberately, rather than reading every result during ordinary navigation.
    const candidates = workflow?.tasks.filter((task) => task.id !== selected && (task.state === 'completed' || task.state === 'queued' && task.started_at === null)) ?? [];
    const active = () => mounted && !owner.disposed && discovery === correctionDiscovery;
    try {
      for (let offset = 0; offset < candidates.length; offset += 4) {
        const results = await Promise.allSettled(candidates.slice(offset, offset + 4).map((task) => client.request<TaskReadDto>(`${workflowPath}/tasks/${encodeURIComponent(task.id)}`)));
        if (!active()) return;
        for (const result of results) {
          if (result.status === 'rejected') throw result.reason;
          cacheTask(owner, result.value);
        }
      }
    } catch (failure) { if (active()) { correctionError = 'Could not load every correction and future consumer on this page. Retry before choosing.'; report(failure); } }
    finally { if (active()) correctionsLoading = false; }
  }
  function corrects(task: TaskReadDto, original: string): boolean {
    const seen: string[] = [];
    let target = task.definition.corrective_of;
    while (target && !seen.includes(target)) { if (target === original) return true; seen.push(target); target = canvas.details.find((item) => item.id === target)?.definition.corrective_of ?? null; }
    return false;
  }
  function adoptCorrection() { if (!selectedTask || !correction) return; stage({ kind: 'adopt_correction', task: selectedTask.id, correction, redirects: redirectCandidates.filter((item) => redirectChoices.includes(`${item.consumer}:${item.input}`)).map(({ consumer, expected_version, input }) => ({ consumer, expected_version, input })) }); correctionOpen = false; }

  function supplyShared() {
    attempt(() => {
      const name = selectedSource?.sharedName ?? (canvas.selected.startsWith('supplied:') ? canvas.selected.slice('supplied:'.length) : null);
      if (!name) return;
      if (workflow && workflow.task_count > 200) throw new Error('This workflow exceeds the canvas’s 200-task review bound. Shared input replacement is unavailable here because every affected future task must be reviewed. Its current value and all drafts are intact.');
      if (workflow && (canvas.topology?.tasks.length ?? 0) < workflow.task_count) throw new Error('Load the remaining connection pages before replacing this shared input, so every affected future task can be reviewed.');
      if (canvas.target.kind === 'existing' && !hasChanges(canvas) && workflow) canvas.target.generation = workflow.generation;
      stageChange(canvas, { kind: 'bind_input', name, value: { kind: 'text', text: supplyText } });
      canvas.notice = 'Input replacement added. Started tasks keep the input they already received.';
    });
  }

  function clearCache() {
    loadGeneration++; loading = false; canvas.details = []; canvas.error = '';
    canvas.notice = 'Cached task details cleared. Select a task to reload it. Drafts and accepted connections are unchanged.';
  }
  const removableReferences = $derived(unusedCanvasReferences(canvas, workflow));
  function removeOffPageReferences() {
    attempt(() => {
      const removed = removeUnusedCanvasReferences(canvas, workflow);
      canvas.notice = `Removed ${removed.wires.length} off-page presentation wires, ${removed.sources.length} unused input cards, and ${removed.aliases.length} unused task references. Accepted tasks and results are unchanged.`;
    });
  }
  const offPagePositions = $derived.by(() => {
    const keep = new Set([...graph.nodes.map((node) => node.id), ...canvas.tasks.map((task) => task.id), ...canvas.sources.map((source) => source.id)]);
    return Object.keys(canvas.positions).filter((id) => !keep.has(id));
  });
  function removeOffPagePositions() {
    attempt(() => {
      const remove = new Set(offPagePositions);
      canvas.positions = Object.fromEntries(Object.entries(canvas.positions).filter(([id]) => !remove.has(id)));
      canvas.notice = `Removed ${remove.size} off-page positions. Current cards and pending work keep their positions. Other pages will use automatic placement when reopened.`;
    });
  }

  async function downloadResult(task: TaskReadDto) { try { saveBlob(`${task.definition.title}-result.txt`, await client.download(`/v1/workflows/${encodeURIComponent(workflow!.id)}/tasks/${encodeURIComponent(task.id)}/result/content`)); } catch (failure) { report(failure); } }
  const submitLabel = $derived(canvas.tasks.some((task) => task.version !== null) || canvas.changes.length ? 'Apply changes' : 'Run pending plan');
  const lanes = $derived(executionLanes(workflow?.tasks ?? [], workflow?.updated_at ?? Date.now()));
  function taskLabel(id: string) { const index = graph.nodes.filter((node) => node.kind === 'task').findIndex((node) => node.id === id); const title = graph.nodes.find((node) => node.id === id)?.label ?? canvas.details.find((task) => task.id === id)?.definition.title ?? 'Task outside this page'; return `${title}${graph.nodes.filter((node) => node.label === title).length > 1 ? ` · Task ${index + 1}` : ''}`; }
  function capabilityLabel(capability: TaskCapability) { const metadata = authoring(session, capability); const label = metadata?.label ?? capability.id; return `${label}${session.authoring.filter((item) => item.label === label).length > 1 ? ` · ${capability.id}` : ''}${metadata?.category === 'process' ? ' · Process' : ''}`; }
</script>

<svelte:window onkeydown={(event) => { if (event.key === 'Escape' && !event.defaultPrevented && event.target instanceof Node && inspector?.contains(event.target) && !(event.target instanceof HTMLSelectElement)) { event.preventDefault(); void closeInspector(); } }} />

<section class="canvas-workspace" aria-label="Workflow workspace">
  <header class="canvas-toolbar"><div class="canvas-title"><p class="eyebrow">{canvas.target.kind === 'new' ? 'Unsaved workflow' : session.daemon_name}</p>{#if titleEditing}<label><span class="visually-hidden">Workflow name</span><input aria-label="Workflow name" bind:value={titleDraft} maxlength="256" onblur={renameWorkflow} onkeydown={(event) => { if (event.key === 'Enter') { event.preventDefault(); void renameWorkflow(); } else if (event.key === 'Escape') titleEditing = false; }} /></label>{:else}<h1><button class="workflow-title-button" disabled={canvas.busy || !!canvas.pending} title="Rename workflow" onclick={() => { titleDraft = workflow?.title ?? ''; titleEditing = true; void tick().then(() => document.querySelector<HTMLInputElement>('[aria-label="Workflow name"]')?.focus()); }}>{workflow?.title ?? (canvas.target.kind === 'existing' ? canvas.target.title : canvas.title || 'New workflow')}</button></h1>{/if}</div><div class="canvas-toolbar-actions"><button disabled={!ready} onclick={() => { usualOpen = !usualOpen; usualChoice = canvas.usualModel ? capabilityKey(canvas.usualModel.capability) : ''; }}>Usual model{canvas.usualModel ? `: ${capabilityLabel(canvas.usualModel.capability)}` : '…'}</button><button class="quiet" onclick={() => { canvas.selected = 'workflow'; canvas.inspector = true; connection = null; edge = null; }}>Workflow</button>{#if workflow && !workflow.wait_question}<button onclick={() => control({ kind: 'wait', question: 'Paused by you.' })} disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>Pause workflow</button>{/if}<button class="primary" data-add-task draggable="true" ondragstart={(event) => event.dataTransfer?.setData('application/x-milkdrift-task', 'task')} onclick={() => addTask()} disabled={!ready || canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>+ Add task</button><button onclick={() => addText()} disabled={!ready || canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>+ Text</button><button onclick={() => attachFile()} disabled={!ready || canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>Attach file</button></div></header>
  {#if workflow?.wait_question}
    <div class="workflow-pause" role="region" aria-label="Workflow pause">
      <div><strong>Workflow paused</strong><p>{workflow.wait_question}</p><small>New work waits for Resume. Work that has already entered may continue; supplying input does not resume it.</small></div>
      <button disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => control({ kind: 'continue' })}>Resume workflow</button>
    </div>
  {/if}
  <input bind:this={attachment} class="visually-hidden" tabindex="-1" aria-label="Attach text file" type="file" accept="text/*,.md,.toml,.json,.csv,.rs,.txt" onchange={(event) => { const file = event.currentTarget.files?.[0]; if (file) void captureFile(file, attachmentTarget); event.currentTarget.value = ''; }} />
  {#if currentProject && canvas.project && currentProject.generation !== canvas.project.generation && canvas.tasks.some((task) => task.definition.execution?.kind === 'project')}
    <div class="notice warning canvas-message">Project settings changed. Review {currentProject.settings.environment.kind === 'host' ? 'host file tools' : currentProject.settings.environment.kind} with {currentProject.settings.allow_changes ? 'read and write' : 'read-only'} access before submitting.
      <button disabled={canvas.busy || !!canvas.pending} onclick={() => attempt(() => { canvas.project = copy(currentProject!); for (const task of canvas.tasks) if (task.definition.execution?.kind === 'project') task.definition.execution.expected_generation = currentProject!.generation; canvas.conflict = false; })}>Use reviewed project settings</button>
    </div>
  {/if}
  {#if usualOpen}{@const unassigned = canvas.tasks.filter((task) => task.version === null && !task.definition.capability.id)}<div class="usual-model-picker" role="region" aria-label="Usual model"><label>Model for this canvas<select bind:value={usualChoice}><option value="">Choose a model or agent</option>{#each session.capabilities as capability (capabilityKey(capability))}<option value={capabilityKey(capability)}>{capabilityLabel(capability)}</option>{/each}</select></label><p>{unassigned.length} unassigned tasks. Existing model choices stay unchanged.</p><button class="primary" disabled={!usualChoice} onclick={() => attempt(() => { useUsualModel(canvas, session, usualChoice, true); usualOpen = false; })}>Use for these {unassigned.length} tasks and new tasks</button><button disabled={!usualChoice} onclick={() => attempt(() => { useUsualModel(canvas, session, usualChoice, false); usualOpen = false; })}>New tasks only</button><button class="quiet" onclick={() => usualOpen = false}>Cancel</button></div>{/if}
  <div class="canvas-body" class:inspector-visible={canvas.inspector}>
    {#if canvas.inspector}
      <aside class="canvas-inspector" aria-label="Selected item" bind:this={inspector}>
        <div class="inspector-top"><span class="eyebrow">{connection ? 'Connect' : edge ? 'Connection' : draftTask ? 'Draft task' : selectedTask ? 'Task' : selectedSource || selectedNode?.kind === 'source' ? 'Input' : 'Workflow'}</span><button class="quiet" aria-label="Close details" onclick={closeInspector}><span class="inspector-close-label">Done</span><span aria-hidden="true">×</span></button></div>
        {#if connection}
          <h2>{connection.order ? 'Run after…' : 'Connect to…'}</h2><p class="field-hint">From {taskLabel(connection.source)}</p>
          <label>Destination task<select value={connection.target} onchange={(event) => chooseDestination(event.currentTarget.value)}><option value="">Choose a task</option>{#each graph.nodes.filter((node) => node.kind === 'task' && node.id !== connection!.source) as node (node.id)}<option value={node.id}>{taskLabel(node.id)}</option>{/each}</select></label>
          {#if graph.nodes.find((node) => node.id === connection!.source)?.kind === 'task' && !connection.order}
            <label class="checkbox-label"><input type="checkbox" bind:checked={connection.outcome} /> Use the outcome even if it fails</label>
            {#if !connection.outcome}<label>Result to use<select bind:value={connection.output}><option value="">Choose a result</option>{#each connectionPorts as port (port.name)}<option value={port.name} disabled={'available' in port && !port.available}>{port.label}{port.required ? '' : ' · May be absent'}</option>{/each}</select></label>{#if connectionPorts.find((port) => port.name === connection!.output)?.required === false}<label class="checkbox-label"><input type="checkbox" bind:checked={connection.optional} /> Continue if this optional result is absent</label>{/if}<p class="field-hint">Waits for a successful result. A failed task blocks this input.</p>{:else}<p class="field-hint">Receives the retained outcome after the source settles, including a failure report.</p>{/if}
          {/if}
          {#if connection.target && !connection.order && availableSlots.length > 1 || connection.target && !connection.order && !targetContract?.allows_additional_inputs && availableSlots.length === 1}<label>Connect as<select bind:value={connection.slot}><option value="">Choose an input</option>{#each availableSlots as slot (slot.name)}<option value={slot.name}>{slot.label}</option>{/each}</select></label>{/if}
          {#if connection.target && !targetContract && !connection.order}<p class="notice warning">This wire will stay unfinished until the task has a model or agent.</p>{/if}

          <button class="primary" onclick={confirmConnection} disabled={!connection.target}>Connect</button>
        {:else if edge}
          <h2>{taskLabel(edge.source)} → {taskLabel(edge.target)}</h2><p>{edge.label}</p>
          {@const savedWire = canvas.connections.find((item) => item.id === edge!.id)}
          {#if savedWire}
            {#if savedWire.kind === 'data'}
              {#if !canvas.sources.some((item) => item.id === savedWire.source) && !savedWire.outcome}<label>Output<select value={savedWire.output ?? ''} onchange={(event) => { savedWire.output = event.currentTarget.value || null; }}><option value="">Choose output</option>{#each sourcePorts(savedWire.source) as port (port.name)}<option value={port.name}>{port.label}</option>{/each}</select></label>{/if}
              {@const receiving = canvas.tasks.find((item) => item.id === savedWire.target)}
              {@const receivingPorts = receiving && authoring(session, receiving.definition.capability)}
              {#if receivingPorts?.inputs.length}<label>Input<select value={savedWire.input ?? ''} onchange={(event) => { savedWire.input = event.currentTarget.value || null; }}><option value="">Choose input</option>{#each receivingPorts.inputs as port (port.name)}<option value={port.name}>{port.label}</option>{/each}</select></label>{/if}
              <details><summary>When the source fails or has no result</summary><label class="checkbox-label"><input type="checkbox" bind:checked={savedWire.optional} /> Continue with absence after settlement</label><label class="checkbox-label"><input type="checkbox" bind:checked={savedWire.outcome} /> Receive the outcome, including failure</label></details>
            {:else}<label>Order<select bind:value={savedWire.kind}><option value="after_success">After success</option><option value="after_completion">After completion</option></select></label>{/if}
          {/if}<p class="field-hint">{edge.draft ? 'This connection has not been submitted.' : 'Changing this connection affects only work that has not started.'}</p>
          {#if edge.binding?.kind === 'task_output'}<button onclick={() => attempt(() => { editable(edge!.target); const wire = canvas.connections.find((item) => item.id === edge!.id); if (wire) wire.outcome = true; edge = null; })}>Use the outcome even if it fails</button>{/if}<button class="danger" onclick={deleteEdge}>Remove connection</button>
        {:else if draftTask}
          <form onsubmit={(event) => event.preventDefault()}>
            <fieldset disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>
              <label class="canvas-task-title">Task title<input value={draftTask.definition.title} placeholder="New task" maxlength="256" oninput={(event) => attempt(() => { updateDefinition(canvas, draftTask!, { ...draftTask!.definition, title: event.currentTarget.value }); draftTask!.titleEdited = true; })} /></label>
              <label>What should this task do?<textarea bind:this={instructionsElement} value={draftTask.definition.instructions} required maxlength="16384" rows="5" placeholder="Describe the work and what a useful result looks like." oninput={(event) => attempt(() => updateInstructions(canvas, draftTask!, event.currentTarget.value))}></textarea></label>
              <label class="model-assignment">Model or agent<select value={capabilityKey(draftTask.definition.capability)} onchange={(event) => attempt(() => { chooseCapability(draftTask!, session, event.currentTarget.value); resolveConnections(canvas, session); })} required><option value="">Choose who does this task</option>{#each session.capabilities as capability (capabilityKey(capability))}<option value={capabilityKey(capability)}>{capabilityLabel(capability)}</option>{/each}</select></label>
              {#if contract && draftTask.contractGeneration !== contract.generation}<p class="notice warning">This model or agent changed. Review its current inputs and outputs.</p><button type="button" onclick={() => attempt(() => { chooseCapability(draftTask!, session, capabilityKey(draftTask!.definition.capability)); resolveConnections(canvas, session); })}>Use reviewed model</button>{/if}
              <button type="button" class="text-action" onclick={onmodels}>+ Add model or agent</button>
              {#if !session.capabilities.length}<p class="notice warning">Connect a model or agent before starting this task.</p>{:else if !contract && draftTask.definition.capability.id}<p class="notice warning">This selection’s input contract is unavailable. Refresh Models & agents; the draft is retained.</p>{/if}
              <div class="input-actions"><button type="button" onclick={() => addText(draftTask!.id)}>+ Add input</button><button type="button" class="quiet" onclick={() => attachFile(draftTask!.id)}>Attach file</button></div>
              {#if selectedEdges.length}<ul class="inspector-connections">{#each selectedEdges as input (input.id)}<li><button type="button" onclick={() => inspectEdge(input)}>{graph.nodes.find((node) => node.id === input.source)?.label}<small>{input.label}</small></button></li>{/each}</ul>{/if}
              <details><summary>More actions</summary><button type="button" onclick={() => addText(draftTask!.id, true)}>Wait for a required input</button><label>Run after<select value="" onchange={(event) => { if (event.currentTarget.value) attempt(() => { addConnection(canvas, session, event.currentTarget.value, draftTask!.id, null, null, 'after_success'); }); }}><option value="">Choose a prerequisite task</option>{#each graph.nodes.filter((node) => node.kind === 'task' && node.id !== draftTask!.id) as node (node.id)}<option value={node.id}>{taskLabel(node.id)}</option>{/each}</select></label><label class="checkbox-label"><input type="checkbox" bind:checked={draftTask.definition.required} /> This task must succeed for the workflow to finish</label></details>
              <TaskPermissions definition={draftTask.definition} {session} project={canvas.project} onchange={(definition) => attempt(() => updateDefinition(canvas, draftTask!, definition))} onerror={(message) => canvas.error = message} />
              {#if draftTask.definition.corrective_of}<p class="field-hint">Corrects {taskLabel(draftTask.definition.corrective_of)}. The original result remains in history.</p>{/if}
            </fieldset>
            <div class="inspector-footer"><button class="quiet" type="button" onclick={() => discardTask(draftTask!)} disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload}>Discard {draftTask.version === null ? 'task' : 'edit'}</button></div>
          </form>
        {:else if selectedTask}
          <h2>{selectedTask.definition.title}</h2><span class="badge {selectedTask.state}">{stateLabel(selectedTask)}</span>
          <p class="assigned-model">{authoring(session, selectedTask.definition.capability)?.label ?? selectedTask.definition.capability.id}</p>
          {#if selectedTask.result}<h3>Result</h3>{#if !resultOpen}<p class="result-preview">{selectedTask.result.text.slice(0, 600) || 'No displayable text was retained.'}</p>{/if}<div class="input-actions"><button onclick={() => resultOpen = !resultOpen}>{resultOpen ? 'Hide result' : 'Read result'}</button><button onclick={() => beginConnection(selectedTask!.id)}>Use result</button></div>{#if resultOpen}<pre class="result-content" data-testid="result-output">{selectedTask.result.text}</pre>{#if selectedTask.result.text_truncated}<p class="field-hint">This is a bounded preview.</p>{/if}<button onclick={() => downloadResult(selectedTask!)}>Download result</button>{/if}
          {:else if selectedTask.result_restricted}<p class="notice warning">This connection cannot read the retained result.</p>{:else}<p class="field-hint">No result has been retained yet.</p>{/if}
          <ExecutionDetails definition={selectedTask.definition} project={currentProject ?? canvas.project} {session} />
          {#if workflow?.wait_question && selectedTask.state === 'queued'}<p class="notice warning">Workflow paused</p>{:else if selectedTask.reason}<p class="notice {selectedTask.state === 'failed' ? 'error' : 'warning'}">{readableReason(selectedTask.reason)}</p>{/if}
          {#if selectedTask.frozen && !selectedTask.frozen.entered && ['queued', 'running'].includes(selectedTask.state)}<p class="field-hint">Instructions and inputs are fixed. External entry has not been recorded; the daemon will continue this same task when it is eligible.</p>{/if}
          {#if selectedTask.stop_requested && selectedTask.state !== 'stopped'}<p class="field-hint">Stop requested. The daemon has not reported that this task stopped.</p>{/if}
          <p class="task-instructions">{selectedTask.definition.instructions.slice(0, 700)}</p>
          {#if selectedTask.definition.instructions.length > 700}<details><summary>All instructions</summary><pre>{selectedTask.definition.instructions}</pre></details>{/if}
          {#if selectedEdges.length}<h3>Inputs & prerequisites</h3><ul class="inspector-connections">{#each selectedEdges as input (input.id)}<li><button onclick={() => inspectEdge(input)}>{graph.nodes.find((node) => node.id === input.source)?.label}<small>{input.label}</small></button></li>{/each}</ul>{/if}
          <div class="inspector-actions">{#if selectedTask.state === 'queued' && selectedTask.started_at === null}<button onclick={() => attempt(() => editable(selectedTask!.id))}>Edit task</button><button class="danger" onclick={() => stage({ kind: 'remove', task: selectedTask!.id, expected_version: selectedTask!.version })}>Remove unstarted task</button>{/if}{#if ['queued', 'running', 'uncertain'].includes(selectedTask.state)}<button class="danger" disabled={selectedTask.stop_requested || canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => control({ kind: 'stop', task: selectedTask!.id })}>Request stop</button>{/if}<button onclick={() => createCorrection(selectedTask!)}>Create correction</button>{#if ['failed', 'stopped'].includes(selectedTask.state)}<button onclick={() => { if (correctionOpen) { correctionOpen = false; correctionDiscovery++; correctionsLoading = false; } else void discoverCorrections(); }}>Use correction for future work</button>{/if}</div>
          {#if correctionOpen}{#if correctionsLoading}<p role="status">Loading corrections and future consumers on this task page…</p>{:else if correctionError}<p class="notice warning">{correctionError}</p><button onclick={discoverCorrections}>Retry correction discovery</button>{/if}<label>Successful correction<select disabled={correctionsLoading || !!correctionError} bind:value={correction}><option value="">Choose completed corrective work</option>{#each canvas.details.filter((task) => task.state === 'completed' && corrects(task, selectedTask!.id)) as task (task.id)}<option value={task.id}>{taskLabel(task.id)}</option>{/each}</select></label><p class="field-hint">Select future tasks to receive the correction’s result. Started inputs remain unchanged.</p>{#each redirectCandidates as candidate (`${candidate.consumer}:${candidate.input}`)}<label class="checkbox-label"><input type="checkbox" value={`${candidate.consumer}:${candidate.input}`} bind:group={redirectChoices} /> {candidate.label}</label>{/each}{#if workflow && canvas.details.length < workflow.task_count}<p class="notice warning">Only corrections and future consumers from loaded task pages are listed. Load other task pages to inspect additional choices.</p>{/if}<button disabled={!correction || correctionsLoading || !!correctionError} onclick={adoptCorrection}>Use selected correction</button>{/if}
          {#each workflow?.admissions.filter((admission) => admission.task_id === selectedTask!.id && admission.status === 'rejected') ?? [] as admission (admission.task_id)}<div class="notice warning"><h3>Follow-up proposal needs review</h3><p>{admission.reason}</p><p class="field-hint">Add any repaired tasks and connections to this drawing, then dismiss the proposal in the same submission. This does not resume a separate human wait.</p><button onclick={() => stage({ kind: 'dismiss_admission', task: admission.task_id })}>Dismiss reviewed proposal</button></div>{/each}
          <details><summary>Timing & history</summary><dl class="facts"><dt>Inputs fixed at start</dt><dd>{selectedTask.started_at === null ? 'Not started' : timeLabel(selectedTask.started_at)}</dd><dt>External entry</dt><dd>{selectedTask.frozen?.entered ? 'Entry intent recorded; outcome reported separately' : 'No entry recorded'}</dd><dt>Settled</dt><dd>{selectedTask.ended_at === null ? 'No settlement recorded' : timeLabel(selectedTask.ended_at)}</dd></dl>{#if selectedTask.definition.corrective_of}<p>Corrects {taskLabel(selectedTask.definition.corrective_of)}</p>{/if}{#if selectedTask.origin}<p>Created by a follow-up proposal from {taskLabel(selectedTask.origin.task_id)}.</p>{/if}</details>
          {#if selectedTask.frozen}<details><summary>Inputs used at start</summary>{#each Object.entries(selectedTask.frozen.inputs) as [name, input] (name)}<h3>{input.name ?? 'Captured input'}</h3>{#if input.restricted}<p>Access restricted.</p>{:else if input.text !== null}<pre>{input.text}</pre>{:else}<p>{input.kind === 'absent' ? 'Explicitly absent' : 'Captured immutable input'}</p>{/if}{#if input.text_truncated}<p class="field-hint">Bounded preview; the complete captured bytes are unchanged.</p>{/if}{/each}</details>{/if}
          <details><summary>Technical diagnostics</summary><dl class="facts"><dt>Task</dt><dd data-testid="task-id">{selectedTask.id}</dd><dt>Version</dt><dd>{selectedTask.version}</dd>{#if selectedTask.reason}<dt>Daemon reason</dt><dd>{selectedTask.reason}</dd>{/if}<dt>Result digest</dt><dd>{selectedTask.result?.digest ?? 'No result'}</dd></dl></details>
        {:else if selectedSource}
          <h2>{selectedSource.label}</h2>{#if sourceEdges.some((item) => !item.draft) && !selectedSource.sharedName}<p class="field-hint">Editing this source affects pending tasks and new connections. Accepted tasks retain their current input.</p>{/if}
          {#if selectedSource.sharedName && (sourceEdges.some((item) => !item.draft) || canvas.topology?.tasks.some((task) => Object.values(task.inputs).some((input) => input.kind === 'supplied' && input.source === `supplied:${selectedSource!.sharedName}`)))}<p class="field-hint">Shared input used by future tasks. Started tasks keep their captured value.</p><p class="field-hint">Affected future tasks: {sourceEdges.filter((item) => workflow?.tasks.some((task) => task.id === item.target && task.state === 'queued' && task.started_at === null)).map((item) => taskLabel(item.target)).join(', ') || 'none on this page'}.</p><label>Replacement text<textarea disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} bind:value={supplyText} rows="4" maxlength={session.input_limit_bytes}></textarea></label><button disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={supplyShared}>Supply input</button>{:else if selectedSource.value?.kind === 'uploaded'}<p class="field-hint">A captured file. This does not grant access to the daemon’s filesystem.</p>{:else}<label>Input text<textarea disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} value={selectedSource.text} rows="7" maxlength={session.input_limit_bytes} oninput={(event) => attempt(() => { updateSource(canvas, selectedSource!, event.currentTarget.value, session.input_limit_bytes); })}></textarea></label><label class="checkbox-label"><input type="checkbox" disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} checked={selectedSource.deferred} onchange={(event) => attempt(() => { updateSource(canvas, selectedSource!, selectedSource!.text, session.input_limit_bytes, event.currentTarget.checked); })} /> Supply this input later</label>{/if}
          <button onclick={() => beginConnection(selectedSource!.id)}>Connect to…</button>
          {#if sourceEdges.length}<h3>Used by</h3>{#each sourceEdges as item (item.id)}<button class="connection-row" onclick={() => select(item.target)}>{taskLabel(item.target)}</button>{/each}{:else}<p class="field-hint">Not connected yet. Connect it to the task that should receive it.</p><button class="quiet" disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => attempt(() => { assertEditable(canvas); const id = selectedSource!.id; canvas.sources = canvas.sources.filter((source) => source.id !== id); delete canvas.positions[id]; canvas.inspector = false; })}>Discard input</button>{/if}
        {:else if selectedNode?.kind === 'source'}
          <h2>{selectedNode.label}</h2><p>{selectedNode.detail}</p>{#if sourceEdges[0]?.preview?.value?.text}<pre>{sourceEdges[0].preview.value.text}</pre>{/if}
          {#if sourceEdges[0]}{@const item = sourceEdges[0]}{@const task = canvas.details.find((task) => task.id === item.target)}{@const binding = item.binding}{@const captured = item.input ? binding?.kind === 'supplied' ? (workflow?.supplied_inputs?.[binding.name] ?? task?.supplied_inputs?.[binding.name])?.value : task?.inline_inputs?.[item.input]?.value : null}{#if captured?.text}<pre>{captured.text}</pre>{:else if binding?.kind === 'inline' && binding.value.kind === 'text'}<pre>{binding.value.text}</pre>{/if}{/if}
          <h3>Used by</h3>{#each sourceEdges as item (item.id)}<button class="connection-row" onclick={() => select(item.target)}>{taskLabel(item.target)}</button>{/each}
          {#if canvas.selected.startsWith('supplied:')}
            <h3>Supply input</h3><p class="field-hint">Changes apply to future starts. Tasks that already started keep their captured input.</p>
            {@const suppliedName = canvas.selected.slice('supplied:'.length)}
            {@const consumers = canvas.topology?.tasks.filter((task) => !workflow?.tasks.find((item) => item.id === task.id)?.started_at && Object.values(task.inputs).some((input) => input.kind === 'supplied' && input.source === `supplied:${suppliedName}`)) ?? []}
            <p class="field-hint">Affected future tasks: {consumers.map((task) => taskLabel(task.id)).join(', ') || 'None on loaded pages'}.</p>
            <label>Replacement text<textarea disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} bind:value={supplyText} maxlength={session.input_limit_bytes} rows="4"></textarea></label><button disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={supplyShared}>Supply input</button>
          {/if}
        {:else if selectedNode?.kind === 'boundary'}<h2>Task outside this page</h2><p class="field-hint">This connection points to retained work outside the loaded task page. It is not a missing task. Use the task page controls to inspect more work.</p>
        {:else if canvas.selected === 'workflow'}
          <h2>{workflow?.title ?? 'New workflow'}</h2>{#if !workflow}<label>Workflow name<input disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} bind:value={canvas.title} placeholder="Suggested from the first task" maxlength="256" /></label><label class="checkbox-label"><input type="checkbox" disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} bind:checked={canvas.finishWhenSettled} /> Finish when required work settles</label>{:else}<span class="badge {workflow.state}">{workflow.state}</span>{#if workflow.wait_question}<p class="notice warning">{workflow.wait_question}</p>{/if}<div class="inspector-actions"><button disabled={!!workflow.wait_question || canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => pauseOpen = !pauseOpen}>Wait for a decision</button><button disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => stage({ kind: 'finish' })}>Finish when work settles</button><button disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => { limits = copy(workflow!.limits); limitsOpen = !limitsOpen; }}>Review limits</button><button onclick={onrefresh}>Refresh</button></div>{/if}
          {#if pauseOpen}<label>What needs a decision?<textarea bind:value={waitQuestion} rows="3" maxlength="4096"></textarea></label><button disabled={!waitQuestion.trim() || canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => { void control({ kind: 'wait', question: waitQuestion }); pauseOpen = false; }}>Wait for this decision</button>{/if}
          {#if limitsOpen && limits}<p class="field-hint">These limits apply prospectively. Past spending and unresolved work remain accounted for.</p>{#each Object.entries(limitLabels) as [name, label] (name)}<label>{label}<input type="number" min="1" step="1" bind:value={limits[name as keyof WorkflowLimitsDto]} /></label>{/each}<button onclick={() => { if (limits) stage({ kind: 'set_limits', limits }); limitsOpen = false; }}>Use these limits</button>{/if}
          {#if workflow}<details><summary>Current limits & spending</summary><dl class="facts"><dt>Tasks</dt><dd>{workflow.admitted_tasks}/{workflow.limits.tasks}</dd><dt>Follow-up batches</dt><dd>{workflow.admitted_model_batches}/{workflow.limits.model_batches}</dd><dt>Rejected responses</dt><dd>{workflow.rejected_responses}/{workflow.limits.rejected_responses}</dd><dt>Execution time</dt><dd>{workflow.active_execution_ms}/{workflow.limits.active_execution_ms} ms</dd></dl></details>{/if}
          <p class="field-hint">One Run can include up to {canvas.bounds.run_actions} changes. Larger plans remain saved for deliberate separate runs. The daemon saves unfinished work without running it.</p>
          {#if canvas.details.length}<button class="quiet" onclick={clearCache}>Clear cached details</button>{/if}
          {#if removableReferences.wires.length || removableReferences.aliases.length}<p class="field-hint">Remove saved presentation wires and their unused input cards for accepted work outside this page. Accepted tasks and results stay intact. Pending work, connected sources, and standalone ideas remain.</p><button class="quiet" disabled={!ready || canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={removeOffPageReferences}>Remove off-page canvas references</button>{/if}
          {#if offPagePositions.length}<p class="field-hint">{offPagePositions.length} saved positions belong to cards outside this page. Removing these positions keeps tasks, inputs, connections, and current placements intact.</p><button class="quiet" disabled={!ready} onclick={removeOffPagePositions}>Remove off-page positions</button>{/if}
        {:else}<p class="field-hint">{loading ? 'Loading task details…' : 'Select a card on the canvas.'}</p>{/if}
      </aside>
    {/if}
    <div class="canvas-view">
      <div class="canvas-view-tabs" aria-label="Task presentation"><button aria-pressed={canvas.presentation === 'canvas'} onclick={() => canvas.presentation = 'canvas'}>Canvas</button><button aria-pressed={canvas.presentation === 'list'} onclick={() => canvas.presentation = 'list'}>List</button><button aria-pressed={canvas.presentation === 'timeline'} onclick={() => canvas.presentation = 'timeline'}>Timeline</button></div>
      {#if canvas.presentation === 'canvas'}<WorkflowCanvas bind:canvas nodes={graph.nodes} edges={graph.edges} onselect={select} onconnect={directConnection} ondropfile={(file, target) => captureFile(file, target)} onedge={inspectEdge} onplace={addTask} />
      {:else if canvas.presentation === 'list'}<div class="canvas-list" role="region" aria-label="Tasks and inputs">{#each graph.nodes as node (node.id)}<article><button class="list-node" aria-pressed={canvas.selected === node.id} onclick={() => select(node.id)}><strong>{node.label}</strong><span class="badge {node.state}">{node.draft ? 'Draft' : node.status ?? node.state}</span><p>{node.detail}</p></button>{#if node.kind !== 'boundary'}<button class="quiet" onclick={() => beginConnection(node.id)}>Connect to…</button>{/if}</article>{/each}<h2>Connections</h2>{#each graph.edges as item (item.id)}<button onclick={() => inspectEdge(item)}>{taskLabel(item.source)} → {taskLabel(item.target)} · {item.label}</button>{/each}</div>
      {:else}<div class="canvas-timeline" role="region" aria-label="Recorded task lifetimes"><p class="field-hint">Recorded starts to settlements, including preparation and waiting before entry. A start fixes instructions and inputs; it does not prove external execution began.</p>{#each lanes as lane (lane.task.id)}<div class="execution-row"><button class="quiet execution-label" onclick={() => select(lane.task.id)}>{lane.task.title}</button><div class="execution-track"><button class="execution-bar" class:open={lane.task.ended_at === null} style={`left:${lane.left}%;width:${Math.max(.5, lane.width)}%`} aria-label={`${lane.task.title}: start recorded ${timeLabel(lane.task.started_at)}, ${lane.task.entered ? 'external entry recorded' : 'no external entry recorded'}, ${lane.task.ended_at === null ? 'no end recorded' : `ended ${timeLabel(lane.task.ended_at)}`}`} onclick={() => select(lane.task.id)}></button></div></div>{:else}<p>No tasks have a recorded start yet.</p>{/each}</div>{/if}
      {#if !graph.nodes.length}<div class="canvas-empty"><span aria-hidden="true">✦</span><h2>{workflow ? 'No tasks on this page' : 'Give your first task some space'}</h2><p>Describe the work. Connect inputs when you need them.</p><button class="primary" onclick={() => addTask()}>Add a task</button></div>{/if}
      {#if graph.truncated}<p class="graph-boundary">This drawing exceeds the 600-card or 1,024-connection display bound. Refine the loaded page before editing; undisplayed connections are retained.</p>{/if}
      {#if workflow && (canvas.topology?.next_cursor || topologyCursor)}<div class="topology-pages"><span>Connections are shown for the loaded page. Other tasks remain intact.</span>{#if topologyCursor}<button disabled={loading} onclick={() => loadPage(workflow!, null)}>First connection page</button>{/if}{#if canvas.topology?.next_cursor}<button disabled={loading} onclick={() => loadPage(workflow!, canvas.topology!.next_cursor)}>Next connection page</button>{/if}</div>{/if}
      {#if workflow && workflow.task_count > workflow.tasks.length}<div class="graph-boundary"><span>Showing {workflow.tasks.length} of {workflow.task_count} tasks. Off-page connections stay intact.</span><button onclick={() => onpage()}>First task page</button>{#if workflow.next_task_cursor}<button onclick={() => onpage(workflow!.next_task_cursor!)}>Next task page</button>{/if}</div>{/if}
    </div>
  </div>
  <footer class="canvas-status">
    <div aria-live="polite"><span class="save-state">{!ready ? 'Loading saved canvas…' : canvas.authoringSave.status === 'saved' && canvas.positionSave.status === 'saved' ? 'Saved' : canvas.authoringSave.status === 'saving' || canvas.positionSave.status === 'saving' ? 'Saving…' : 'Not saved'}</span>{#if canvas.notice}<span>{canvas.notice}</span>{:else}<span>New work runs when you choose Run.</span>{/if}</div>
    {#if changeCount(canvas) && !canvas.pending}<div class="drawing-actions"><button class="quiet" disabled={canvas.busy || !ready} onclick={() => { scopeOpen = !scopeOpen; if (scope === null) scope = canvas.tasks.map((task) => task.id); }}>Choose tasks to run</button><button class="primary" disabled={canvas.busy || canvas.conflict || !!canvas.pendingUpload || !changeCount(canvas) || !ready} onclick={() => reviewScope(true)}>{canvas.busy ? 'Working…' : submitLabel}</button></div>{/if}
  </footer>
  {#if scopeOpen}<section class="run-scope" aria-label="Tasks to run"><div class="inspector-top"><h2>Tasks to run</h2><button class="quiet" aria-label="Close run scope" onclick={() => scopeOpen = false}>×</button></div><p>Only this named plan will run. Other ideas stay on the canvas.</p>{#each canvas.tasks as task (task.id)}<label class="checkbox-label"><input type="checkbox" checked={(scope ?? canvas.tasks.map((item) => item.id)).includes(task.id)} onchange={(event) => { const current = scope ?? canvas.tasks.map((item) => item.id); scope = event.currentTarget.checked ? [...new Set([...current, task.id])] : current.filter((id) => id !== task.id); if (event.currentTarget.checked) useCurrent = useCurrent.filter((id) => id !== task.id); review = null; }} />{task.definition.title || 'New task'}{task.version !== null ? ' · Apply candidate edit' : ''}</label>{/each}{#if review}{#each review.issues as issue, index (index)}<div class="notice warning"><p>{issue.message}</p>{#if issue.code === 'predecessor_choice' && issue.task_id}<button onclick={() => { useCurrent = [...new Set([...useCurrent, issue.task_id!])]; scope = (scope ?? []).filter((id) => id !== issue.task_id); void reviewScope(); }}>Use current task</button><button onclick={() => { scope = [...new Set([...(scope ?? []), issue.task_id!])]; useCurrent = useCurrent.filter((id) => id !== issue.task_id); void reviewScope(); }}>Apply these changes</button>{:else if issue.connection_id}<button onclick={() => { const value = graph.edges.find((item) => item.id === issue.connection_id); if (value) inspectEdge(value); scopeOpen = false; }}>Inspect connection</button>{:else if issue.task_id}<button onclick={() => { select(issue.task_id!); scopeOpen = false; }}>Inspect task</button>{/if}</div>{/each}<p>{review.tasks.length} tasks and {review.changes.length} other changes in the reviewed plan.</p>{/if}<button class="primary" disabled={canvas.busy} onclick={() => review && !review.issues.length ? runReviewed(review) : reviewScope()}>{review && !review.issues.length ? 'Run reviewed plan' : 'Review this plan'}</button></section>{/if}
  {#if canvas.authoringSave.status === 'conflict'}<div class="notice warning canvas-message"><p>The saved canvas changed elsewhere. Your candidate is preserved.</p><details><summary>Compare saved and my canvas</summary>{#each [{ label: 'Saved elsewhere', document: canvas.authoringSave.remote?.document }, { label: 'My canvas', document: canvasDocument(canvas) }] as version (version.label)}<section><h3>{version.label}</h3>{#if version.document}<p>{version.document.tasks.length} pending tasks · {version.document.connections.length} connections · {version.document.sources.length} inputs</p>{#each version.document.tasks as task (task.id)}<details><summary>{task.definition.title || 'New task'}</summary><p>{capabilityLabel(task.definition.capability)}</p><pre>{task.definition.instructions || 'No instructions yet'}</pre></details>{/each}{#each version.document.sources as source (source.id)}<p>{source.label}: {source.value?.kind === 'text' ? source.value.text : source.value?.kind === 'uploaded' ? 'Captured file' : 'Input to supply'}</p>{/each}{/if}</section>{/each}</details><button onclick={() => { const value = checkpoint.useSaved(); if (value) adoptAuthoring(canvas, value); }}>Use saved canvas</button><button onclick={() => { checkpoint.keepCandidate(); void checkpoint.flush(); }}>Keep my canvas</button></div>{:else if canvas.authoringSave.error}<div class="notice warning canvas-message"><p>{canvas.authoringSave.error}</p><button onclick={() => checkpoint.flush()}>Retry save</button></div>{/if}
  {#if canvas.positionSave.status === 'conflict'}<div class="notice warning canvas-message"><p>Positions changed in another client.</p><button onclick={() => { const value = positions.useSaved(); if (value) canvas.positions = value; }}>Use saved positions</button><button onclick={() => { positions.keepCandidate(); void positions.flush(); }}>Keep my positions</button></div>{:else if canvas.positionSave.error}<div class="notice warning canvas-message"><p>Positions are not saved: {canvas.positionSave.error}</p><button onclick={() => positions.flush()}>Retry position save</button></div>{/if}
  {#if canvas.changes.length}<details class="drawing-review"><summary>Changes to apply ({changeCount(canvas)})</summary><ul>{#each canvas.tasks as task (task.id)}<li>{task.version === null ? 'Add' : 'Edit'} {task.definition.title || 'New task'} · {Object.keys(task.definition.inputs).length} inputs</li>{/each}{#each canvas.changes as change, index (index)}<li>{change.kind === 'bind_input' ? 'Supply input for future tasks' : change.kind === 'adopt_correction' ? `Use ${taskLabel(change.correction)} for ${taskLabel(change.task)}` : change.kind.replaceAll('_', ' ')}{'task' in change && change.kind !== 'adopt_correction' ? `: ${taskLabel(change.task)}` : ''}<button class="quiet" disabled={canvas.busy || !!canvas.pending || !!canvas.pendingUpload} onclick={() => canvas.changes = canvas.changes.filter((_, at) => at !== index)}>Undo</button></li>{/each}</ul></details>{/if}
  {#if canvas.error}<p class="notice error canvas-message" role="alert">{canvas.error}</p>{/if}
  {#if canvas.conflict}<div class="notice warning canvas-message">The workflow changed. Your drawing is intact. Review the current tasks before another submission. <button disabled={canvas.busy} onclick={refreshConflict}>Refresh workflow for review</button></div>{/if}
  {#if canvas.pending}<div class="notice warning canvas-message"><p>{canvas.pending.control ? 'The control reply is unconfirmed. Its exact request is retained; your drawing has not been submitted.' : 'The reply is unconfirmed. Its exact destination and drawing are retained.'}</p><button disabled={canvas.busy} onclick={() => submit(true)}>Find submission receipt</button><button disabled={canvas.busy} onclick={() => submit()}>Replay exact submission</button></div>{/if}
  {#if canvas.pendingUpload}<div class="notice warning canvas-message"><p>The file capture reply is unconfirmed.</p><button disabled={canvas.busy} onclick={() => captureFile()}>Retry exact upload</button></div>{/if}
</section>
