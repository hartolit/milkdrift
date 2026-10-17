<script lang="ts">
  import type { CreateWorkflowRequest, TaskActionPolicyDto, TaskSessionDto, TaskToolDto, TaskUploadReceipt, TaskUploadRequest, WorkflowReceipt } from '../lib/protocol.generated';
  import { DaemonClient, DaemonError } from '../lib/transport';
  import TaskModelOptions from './TaskModelOptions.svelte';
  import { captureTextRequest } from '../lib/task-input';

  let { client, session, oncreated, oncancel, onerror }: {
    client: DaemonClient;
    session: TaskSessionDto;
    oncreated: (receipt: WorkflowReceipt) => Promise<void>;
    oncancel: () => void;
    onerror: (error: unknown) => void;
  } = $props();
  let title = $state('');
  let description = $state('');
  let instructions = $state('');
  let brief = $state('');
  let supplyBrief = $state(true);
  let capabilityIndex = $state(0);
  let files: FileList | undefined = $state();
  let upload = $state<TaskUploadReceipt | null>(null);
  let path = $state('');
  let pathWrite = $state(false);
  let actions = $state<TaskActionPolicyDto | null>(null);
  let tools = $state<TaskToolDto[]>([]);
  let finishWhenSettled = $state(true);
  let error = $state('');
  let busy = $state(false);
  let pending = $state<CreateWorkflowRequest | null>(null);
  let pendingUpload = $state<TaskUploadRequest | null>(null);

  function selectCapability(event: Event) {
    capabilityIndex = Number((event.currentTarget as HTMLSelectElement).value);
    if (session.capabilities[capabilityIndex]?.operation !== 'model.generate') {
      actions = null;
      tools = [];
    }
  }

  async function captureFile() {
    const reportError = onerror;
    const file = files?.[0];
    if (!file) return;
    busy = true;
    error = '';
    upload = null;
    pendingUpload = null;
    try {
      pendingUpload = await captureTextRequest(file, session.input_limit_bytes);
      upload = await client.request<TaskUploadReceipt>('/v1/task-inputs', pendingUpload);
      pendingUpload = null;
    } catch (failure) {
      error = failure instanceof Error ? failure.message : 'The file upload failed.';
      reportError(failure);
    } finally { busy = false; }
  }

  async function retryUpload() {
    const reportError = onerror;
    if (!pendingUpload) return;
    busy = true;
    try {
      upload = await client.request<TaskUploadReceipt>('/v1/task-inputs', pendingUpload);
      pendingUpload = null;
      error = '';
    } catch (failure) { error = failure instanceof Error ? failure.message : 'The file upload failed.'; reportError(failure); }
    finally { busy = false; }
  }

  async function submit(event?: SubmitEvent) {
    const reportCreated = oncreated;
    const reportError = onerror;
    event?.preventDefault();
    const capability = session.capabilities[capabilityIndex];
    if (!capability) return;
    if (!pending) {
      pending = {
        request_id: crypto.randomUUID(), title, description, finish_when_settled: finishWhenSettled, limits: null,
        inputs: { ...(supplyBrief ? { brief: { kind: 'text' as const, text: brief } } : {}), ...(upload ? { file: { kind: 'uploaded' as const, input_id: upload.input_id } } : {}) },
        tasks: [{ alias: 'task', definition: {
          title, description, instructions, capability, actions, tools,
          inputs: { brief: { kind: 'supplied', name: 'brief', optional: false }, ...(upload ? { file: { kind: 'supplied' as const, name: 'file', optional: false } } : {}) },
          dependencies: [], outputs: {}, required: true, corrective_of: null,
          paths: path.trim() ? [{ path: path.trim(), write: pathWrite }] : [],
        } }],
      };
    }
    busy = true;
    error = '';
    try {
      const receipt = await client.request<WorkflowReceipt>('/v1/workflows', pending);
      pending = null;
      await reportCreated(receipt);
    } catch (failure) {
      error = failure instanceof Error ? failure.message : 'The submission outcome is unknown.';
      if (failure instanceof DaemonError && failure.status >= 400 && failure.status < 500) pending = null;
      reportError(failure);
    } finally { busy = false; }
  }

  async function findReceipt() {
    const reportCreated = oncreated;
    const reportError = onerror;
    if (!pending) return;
    busy = true;
    try {
      const receipt = await client.request<WorkflowReceipt>(`/v1/workflow-requests/${encodeURIComponent(pending.request_id)}`);
      pending = null;
      await reportCreated(receipt);
    } catch (failure) { error = failure instanceof Error ? failure.message : 'No receipt could be read.'; reportError(failure); }
    finally { busy = false; }
  }
</script>

<section class="panel task-composer" aria-labelledby="task-composer-heading">
  <div class="section-heading"><div><p class="eyebrow">{session.daemon_name} / New work</p><h1 id="task-composer-heading">Start a task</h1></div><button class="icon-button" onclick={oncancel} aria-label="Close task form">×</button></div>
  <p class="muted">Set the outcome, then add the context your task needs. Work continues on your daemon when this page closes.</p>
  <form onsubmit={submit}>
    <fieldset disabled={busy || pending !== null}>
      <div class="authoring-layout">
        <div class="authoring-main">
          <fieldset class="form-section"><legend>Task instructions</legend>
            <p class="field-hint">What should the task do, and what should it produce?</p>
            <label>Task title<input bind:value={title} placeholder="Compare Rust animation approaches" required maxlength="256" /></label>
            <label>Short description<input bind:value={description} placeholder="What this work is for" maxlength="4096" /></label>
            <label>Instructions<textarea bind:value={instructions} placeholder="Explain the work to perform, the expected output, and any constraints." required rows="6"></textarea></label>
          </fieldset>
          <fieldset class="form-section"><legend>Supplied context</legend>
            <p class="field-hint">Provide the brief and source material separately from the task’s instructions.</p>
            <label class="checkbox-label"><input type="checkbox" bind:checked={supplyBrief} /> Supply brief now</label>
            {#if supplyBrief}<label>Brief input<textarea bind:value={brief} placeholder="Project details, a question, or data the task should use." rows="4"></textarea></label>{:else}<p class="notice warning">The task requires a brief. It will stay queued until you supply that input in its inspector.</p>{/if}
            <label>Text file input<input type="file" bind:files accept="text/*,.md,.toml,.json,.csv,.rs,.txt" onchange={captureFile} /></label>
            <p class="field-hint">UTF-8 text up to {(session.input_limit_bytes / 1024).toLocaleString()} KiB. The daemon also checks the combined instructions and selected inputs.</p>
            {#if upload}<p class="notice">Captured {upload.name} · {upload.bytes.toLocaleString()} bytes</p><details class="advanced-options"><summary>Captured file identity</summary><p class="field-hint">This digest identifies the exact file content retained by the daemon.</p><code>{upload.digest}</code></details>{/if}
          </fieldset>
        </div>
        <aside class="authoring-aside" aria-label="Task execution options">
          <fieldset class="form-section"><legend>Execution</legend>
            <label>Capability<select aria-label="Capability" value={capabilityIndex} onchange={selectCapability} required>{#each session.capabilities as capability, index (`${capability.id}/${capability.operation}`)}<option value={index}>{capability.id} · {capability.operation}</option>{/each}</select></label>
            <p class="field-hint">Choose a configured model or tool available on this daemon.</p>
            <label class="checkbox-label"><input type="checkbox" bind:checked={finishWhenSettled} /> Finish when required work settles</label>
            {#if session.capabilities[capabilityIndex]?.operation === 'model.generate'}<TaskModelOptions bind:actions bind:tools capabilities={session.capabilities} />{/if}
          </fieldset>
          <details class="advanced-options"><summary>Project filesystem access</summary><p class="field-hint">Declare a project root for configured processes or model file tools. The daemon validates the path against your grant.</p><label>Absolute project root<input bind:value={path} placeholder="/home/you/projects/animation" /></label><label class="checkbox-label"><input type="checkbox" bind:checked={pathWrite} /> Allow writes within this root</label></details>
          <details class="advanced-options"><summary>Workflow limits</summary><p class="field-hint">These daemon defaults bound the work created by this submission.</p><div class="limits" aria-label="Effective workflow limits">
            <span>{session.limits.tasks} tasks</span><span>{session.limits.concurrent_tasks} concurrent</span>
            <span>{session.limits.model_batches} model batches</span><span>{session.limits.tool_calls_per_task} tool calls per task</span>
            <span>{session.limits.active_execution_ms / 60000} active minutes</span>
            <span>{session.limits.rejected_responses} consecutive rejected responses</span><span>{session.limits.corrective_failures} corrective failures</span>
          </div></details>
        </aside>
      </div>
    </fieldset>
    {#if !session.capabilities.length}<p class="notice warning">No permitted capabilities are available. Ask the daemon administrator to configure a capability and your execution allowance.</p>{/if}
    {#if error}<p class="notice error" role="alert">{error}</p>{/if}
    {#if pendingUpload}<button type="button" onclick={retryUpload} disabled={busy}>Retry exact upload</button>{/if}
    <div class="authoring-footer">{#if pending}
      <p class="notice warning">The submission may have been accepted. Look up its receipt or replay the exact request. Both use request <code>{pending.request_id}</code>.</p>
      <div class="form-actions"><button type="button" class="primary" disabled={busy} onclick={findReceipt}>Find submission receipt</button><button type="button" disabled={busy} onclick={() => submit()}>Replay exact submission</button></div>
    {:else}
      <div class="form-actions"><button class="primary" disabled={busy || !session.capabilities.length || pendingUpload !== null}>{busy ? 'Submitting…' : 'Submit task'}</button><button type="button" class="quiet" onclick={oncancel}>Cancel</button></div>
    {/if}</div>
  </form>
</section>
