<script lang="ts">
  import { onMount } from 'svelte';
  import type { TaskCapability, TaskSessionDto, TaskUploadReceipt, WorkflowReadDto, WorkflowReceipt } from '../lib/protocol.generated';
  import { DaemonClient, DaemonError } from '../lib/transport';
  import TaskDefinitionEditor from './TaskDefinitionEditor.svelte';
  import { captureTextRequest } from '../lib/task-input';
  import { capabilityKey, freezeTaskSubmission, updateDraftContext, updateDraftInstructions, type TaskDraft } from '../lib/task-composer';

  let { client, session, draft = $bindable(), onaccepted, onclose, ondiscard, onerror, oncapabilitychoice, onreviewed }: {
    client: DaemonClient;
    session: TaskSessionDto;
    draft: TaskDraft;
    onaccepted: (receipt: WorkflowReceipt) => Promise<void>;
    onclose: () => void;
    ondiscard: () => void;
    onerror: (error: unknown) => void;
    oncapabilitychoice?: (capability: TaskCapability) => void;
    onreviewed?: (workflow: WorkflowReadDto) => void;
  } = $props();
  let instructionsElement: HTMLTextAreaElement;
  let attachmentElement: HTMLInputElement;
  let composerElement: HTMLElement;
  onMount(() => instructionsElement?.focus());
  const selectedCapability = $derived(session.capabilities.find((item) => capabilityKey(item) === capabilityKey(draft.definition.capability)));
  const contextBinding = $derived(draft.definition.inputs.brief);
  const contextIsCompact = $derived(!contextBinding || (contextBinding.kind === 'inline' && contextBinding.value.kind === 'text') || (contextBinding.kind === 'supplied' && contextBinding.name === `context_${draft.id.replaceAll('-', '')}`));
  const supplyingContext = $derived(!contextBinding || contextBinding.kind === 'inline');
  const attachedFile = $derived(draft.definition.inputs.file?.kind === 'inline' && draft.definition.inputs.file.value.kind === 'uploaded' && draft.definition.inputs.file.value.input_id === draft.upload?.input_id ? draft.upload : null);
  const limitLabels = {
    tasks: 'Total admitted tasks', concurrent_tasks: 'Concurrent tasks', model_batches: 'Total model batches',
    rejected_responses: 'Consecutive rejected responses', corrective_failures: 'Failures on one corrective lineage',
    active_execution_ms: 'Cumulative execution milliseconds', tool_calls_per_task: 'Tool calls per task',
  };

  function selectCapability(event: Event) {
    const capability = session.capabilities.find((item) => capabilityKey(item) === (event.currentTarget as HTMLSelectElement).value);
    if (!capability) return;
    draft.definition.capability = { ...capability };
    if (capability.operation !== 'model.generate') { draft.definition.actions = null; draft.definition.tools = []; }
    oncapabilitychoice?.({ ...capability });
  }

  async function captureFile(event?: Event) {
    const current = draft, connection = client, reportError = onerror;
    if (current.busy || current.disposed || current.pending) return;
    const file = event ? (event.currentTarget as HTMLInputElement).files?.[0] : undefined;
    if (event && !file) return;
    current.busy = true; current.error = '';
    try {
      if (file) {
        const request = await captureTextRequest(file, session.input_limit_bytes);
        if (current.disposed) return;
        current.pendingUpload = request;
      }
      if (!current.pendingUpload) return;
      const captured = await connection.request<TaskUploadReceipt>('/v1/task-inputs', current.pendingUpload);
      if (current.disposed) return;
      current.upload = captured;
      current.definition.inputs = { ...current.definition.inputs, file: { kind: 'inline', value: { kind: 'uploaded', input_id: captured.input_id } } };
      current.pendingUpload = null;
    } catch (failure) {
      if (current.disposed) return;
      current.error = failure instanceof Error ? failure.message : 'The file upload failed.';
      if (failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) current.pendingUpload = null;
      reportError(failure);
    } finally { if (!current.disposed) current.busy = false; }
  }

  async function submit(event?: SubmitEvent) {
    event?.preventDefault();
    const current = draft, connection = client, reportAccepted = onaccepted, reportError = onerror;
    if (current.busy || current.disposed || current.accepted || current.conflict || current.pendingUpload) return;
    current.busy = true; current.error = '';
    try {
      current.pending ??= freezeTaskSubmission(current, session);
      const receipt = await connection.request<WorkflowReceipt>(current.pending.path, current.pending.request);
      if (current.disposed) return;
      current.pending = null; current.accepted = true;
      await reportAccepted(receipt);
    } catch (failure) {
      if (current.disposed) return;
      current.error = failure instanceof Error ? failure.message : 'The submission outcome is unknown.';
      if (failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) {
        current.pending = null;
        current.conflict = current.target.kind === 'existing' && failure.status === 409;
      }
      reportError(failure);
    } finally { if (!current.disposed) current.busy = false; }
  }

  async function findReceipt() {
    const current = draft, connection = client, reportAccepted = onaccepted, reportError = onerror;
    if (!current.pending || current.busy || current.disposed) return;
    const requestId = current.pending.request.request_id;
    current.busy = true; current.error = '';
    try {
      const receipt = await connection.request<WorkflowReceipt>(`/v1/workflow-requests/${encodeURIComponent(requestId)}`);
      if (current.disposed) return;
      current.pending = null; current.accepted = true;
      await reportAccepted(receipt);
    } catch (failure) {
      if (current.disposed) return;
      current.error = failure instanceof Error ? failure.message : 'No receipt could be read.';
      reportError(failure);
    } finally { if (!current.disposed) current.busy = false; }
  }

  async function refreshConflict() {
    const current = draft, connection = client, reportError = onerror, reportReviewed = onreviewed;
    if (current.target.kind !== 'existing' || current.busy || current.disposed) return;
    const workflowId = current.target.workflowId;
    current.busy = true; current.error = '';
    try {
      const workflow = await connection.request<WorkflowReadDto>(`/v1/workflows/${encodeURIComponent(workflowId)}`);
      if (current.disposed || current.target.kind !== 'existing') return;
      current.target.generation = workflow.generation;
      current.conflict = false;
      reportReviewed?.(workflow);
    } catch (failure) {
      if (current.disposed) return;
      current.error = failure instanceof Error ? failure.message : 'The workflow could not be refreshed.';
      reportError(failure);
    } finally { if (!current.disposed) current.busy = false; }
  }
</script>

<svelte:window onkeydown={(event) => {
  if (event.key === 'Escape' && !event.defaultPrevented && event.target instanceof Node && composerElement?.contains(event.target) && !(event.target instanceof HTMLSelectElement)) {
    event.preventDefault(); onclose();
  }
}} />

<section bind:this={composerElement} class="panel task-composer compact-composer" aria-label="Task composer">
  <div class="section-heading"><div><p class="eyebrow">{session.daemon_name} / {draft.target.kind === 'new' ? 'Unsaved workflow' : draft.target.title}</p><h2 id="task-composer-heading">{draft.target.kind === 'new' ? 'New workflow' : 'Add task'}</h2></div><button class="icon-button" onclick={onclose} aria-label="Close composer">×</button></div>
  <p class="field-hint">{draft.target.kind === 'new' ? 'Create a workflow with its first task. Nothing is created until you submit.' : `Add work to ${draft.target.title}. Its current work and completion policy stay in place.`}</p>
  <form onsubmit={submit}>
    <fieldset disabled={draft.busy || draft.pending !== null || draft.accepted}>
      <label>What should this task do?<textarea bind:this={instructionsElement} value={draft.definition.instructions} oninput={(event) => updateDraftInstructions(draft, event.currentTarget.value)} placeholder="Describe the outcome, expected output and any constraints." required maxlength={session.input_limit_bytes} rows="5"></textarea></label>
      <label class="draft-title">Task title<input value={draft.definition.title} oninput={(event) => { draft.definition.title = event.currentTarget.value; draft.titleEdited = true; }} placeholder="Suggested from your instructions" maxlength="256" /></label>
      <label class="executor-choice">Executor<select value={selectedCapability ? capabilityKey(selectedCapability) : ''} onchange={selectCapability} required><option value="" disabled>Choose an executor</option>{#each session.capabilities as capability (capabilityKey(capability))}<option value={capabilityKey(capability)}>{capability.id} · {capability.operation}</option>{/each}</select></label>
      {#if !selectedCapability}<p class="field-hint">{session.capabilities.length ? 'Choose who will perform this task. A changed or unavailable selection is never replaced automatically.' : 'No permitted executors are available. Ask the daemon administrator to configure a capability and your execution allowance.'}</p>{/if}
      <div class="composer-extras"><button type="button" class="quiet" onclick={() => { draft.contextOpen = !draft.contextOpen; }} aria-expanded={draft.contextOpen}>{draft.contextOpen ? 'Hide context' : 'Add context'}</button><button type="button" class="quiet" onclick={() => attachmentElement.click()} disabled={draft.pendingUpload !== null} title={`UTF-8 text, up to ${(session.input_limit_bytes / 1024).toLocaleString()} KiB`}>Attach text file</button><input class="visually-hidden" tabindex="-1" bind:this={attachmentElement} aria-label="Attach text file" type="file" accept="text/*,.md,.toml,.json,.csv,.rs,.txt" onchange={captureFile} disabled={draft.pendingUpload !== null} /></div>
      {#if draft.contextOpen}
        <fieldset class="form-section compact-context"><legend>Optional context</legend>
          {#if contextIsCompact}
            <label class="checkbox-label"><input type="checkbox" checked={supplyingContext} onchange={(event) => { if (contextBinding?.kind === 'inline' && contextBinding.value.kind === 'text') draft.contextText = contextBinding.value.text; draft.supplyContext = event.currentTarget.checked; updateDraftContext(draft); }} /> Supply context now</label>
            {#if supplyingContext}<label>Context<textarea value={contextBinding?.kind === 'inline' && contextBinding.value.kind === 'text' ? contextBinding.value.text : draft.contextText} oninput={(event) => { draft.contextText = event.currentTarget.value; draft.supplyContext = true; updateDraftContext(draft); }} maxlength={session.input_limit_bytes} rows="3" placeholder="Source material, facts or a brief for this task."></textarea></label>
            {:else}<p class="notice warning">This task will wait for its own required context. Supply it later from the task inspector.</p>{/if}
            {#if contextBinding}<button type="button" class="quiet" onclick={() => { const inputs = { ...draft.definition.inputs }; delete inputs.brief; draft.definition.inputs = inputs; draft.contextText = ''; draft.supplyContext = true; }}>Remove context requirement</button>{/if}
          {:else}<p class="field-hint">This task's brief binding uses an advanced rule. Edit it under Named inputs below.</p>{/if}
          <p class="field-hint">Context stays separate from instructions and belongs only to this task. The executor may require named inputs; its daemon validation still applies.</p>
        </fieldset>
      {/if}
      {#if attachedFile}<p class="attached-file">{attachedFile.name} <span class="field-hint">· {attachedFile.bytes.toLocaleString()} bytes</span> <button class="quiet" type="button" onclick={() => { const inputs = { ...draft.definition.inputs }; delete inputs.file; draft.definition.inputs = inputs; draft.upload = null; }}>Remove attachment</button></p>{/if}
      <details class="advanced-options" bind:open={draft.advancedOpen}><summary>Advanced task options</summary>
        <label>Short description<input bind:value={draft.definition.description} maxlength="4096" /></label>
        {#if draft.target.kind === 'new'}<label>Workflow name<input bind:value={draft.workflowTitle} placeholder={draft.definition.title || 'Same as the task title'} maxlength="256" /></label>{/if}
        <TaskDefinitionEditor bind:definition={draft.definition} bind:editor={draft.editor} capabilities={session.capabilities} compact />
        {#if draft.target.kind === 'new'}
          <label class="checkbox-label"><input type="checkbox" bind:checked={draft.finishWhenSettled} /> Finish when required work settles</label>
          <details class="advanced-options"><summary>Workflow limits</summary>
            <label class="checkbox-label"><input type="checkbox" checked={draft.limits !== null} onchange={(event) => draft.limits = event.currentTarget.checked ? { ...session.limits } : null} /> Set tighter workflow limits</label>
            <div class="form-grid">{#each Object.entries(limitLabels) as [name, label] (name)}{#if draft.limits}<label>{label}<input type="number" min="1" max={session.limits[name as keyof typeof limitLabels]} step="1" required bind:value={draft.limits[name as keyof typeof limitLabels]} /></label>{:else}<span>{label}: {session.limits[name as keyof typeof limitLabels]}</span>{/if}{/each}</div>
          </details>
        {/if}
      </details>
    </fieldset>
    {#if draft.target.kind === 'new'}<p class="field-hint">{draft.finishWhenSettled ? 'Finishes when required work settles. You can add more tasks later.' : 'Leaves completion open for a later decision.'}</p>{/if}
    {#if draft.error}<p class="notice error" role="alert">{draft.error}</p>{/if}
    {#if draft.conflict}<p class="notice warning">This workflow changed before the task could be accepted. Your draft is retained. Refresh its current state and review before making a new submission.</p><button type="button" disabled={draft.busy} onclick={refreshConflict}>Refresh workflow for review</button>{/if}
    {#if draft.pendingUpload}<p class="notice warning">The attachment reply is unconfirmed. Recover the exact upload before submitting.</p><button type="button" onclick={() => captureFile()} disabled={draft.busy}>Retry exact upload</button>{/if}
    <div class="authoring-footer">{#if draft.pending}
      <p class="notice warning">The submission may have been accepted. This draft retains its exact destination and request. Look up its receipt or replay it safely.</p>
      <div class="form-actions"><button type="button" class="primary" disabled={draft.busy} onclick={findReceipt}>Find submission receipt</button><button type="button" disabled={draft.busy} onclick={() => submit()}>Replay exact submission</button></div>
    {:else if draft.accepted}<p class="notice">The daemon accepted this task.</p>
    {:else}<div class="form-actions"><button class="primary" disabled={draft.busy || !selectedCapability || draft.pendingUpload !== null || draft.conflict}>{draft.busy ? 'Submitting…' : draft.target.kind === 'new' ? 'Create workflow' : 'Add task'}</button><button type="button" class="quiet" onclick={onclose}>Keep draft</button><button type="button" class="quiet" onclick={ondiscard} disabled={draft.busy || draft.pendingUpload !== null}>Discard draft</button></div>{/if}</div>
  </form>
</section>
