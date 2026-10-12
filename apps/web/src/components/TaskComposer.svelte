<script lang="ts">
  import type { CreateWorkflowRequest, TaskSessionDto, TaskUploadReceipt, TaskUploadRequest, WorkflowReceipt } from '../lib/protocol.generated';
  import { DaemonClient, DaemonError } from '../lib/transport';

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
  let error = $state('');
  let busy = $state(false);
  let pending = $state<CreateWorkflowRequest | null>(null);
  let pendingUpload = $state<TaskUploadRequest | null>(null);

  async function captureFile() {
    const reportError = onerror;
    const file = files?.[0];
    if (!file) return;
    busy = true;
    error = '';
    upload = null;
    pendingUpload = null;
    try {
      if (file.size > session.input_limit_bytes) throw new Error(`This file exceeds the daemon upload bound of ${session.input_limit_bytes.toLocaleString()} bytes. Supply a smaller text file.`);
      const text = new TextDecoder('utf-8', { fatal: true }).decode(await file.arrayBuffer());
      pendingUpload = { request_id: crypto.randomUUID(), name: file.name, media_type: 'text/plain', text };
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
        request_id: crypto.randomUUID(), title, description, finish_when_settled: true, limits: null,
        inputs: { ...(supplyBrief ? { brief: { kind: 'text' as const, text: brief } } : {}), ...(upload ? { file: { kind: 'uploaded' as const, input_id: upload.input_id } } : {}) },
        tasks: [{ alias: 'task', definition: {
          title, description, instructions, capability,
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

<section class="panel" aria-labelledby="task-composer-heading">
  <div class="section-heading"><div><p class="eyebrow">Give work to your daemon</p><h2 id="task-composer-heading">Start a task</h2></div><button class="icon-button" onclick={oncancel} aria-label="Close task form">×</button></div>
  <p class="muted">Describe the outcome and supply its context. The daemon keeps working when you close this page.</p>
  <p class="subheading">Effective daemon defaults</p>
  <div class="limits" aria-label="Effective workflow limits">
    <span>{session.limits.tasks} tasks</span><span>{session.limits.concurrent_tasks} concurrent</span>
    <span>{session.limits.model_batches} model batches</span><span>{session.limits.tool_calls_per_task} tool calls per task</span>
    <span>{session.limits.active_execution_ms / 60000} active minutes</span>
    <span>{session.limits.rejected_responses} consecutive rejected responses</span><span>{session.limits.corrective_failures} corrective failures</span>
  </div>
  <form onsubmit={submit}>
    <fieldset disabled={busy || pending !== null}>
      <div class="form-grid"><label>Task title<input bind:value={title} placeholder="Compare Rust animation approaches" required maxlength="256" /></label><label>Capability<select aria-label="Capability" bind:value={capabilityIndex} required>{#each session.capabilities as capability, index (`${capability.id}/${capability.operation}`)}<option value={index}>{capability.id} · {capability.operation}</option>{/each}</select></label></div>
      <label>Short description<input bind:value={description} placeholder="What this work is for" maxlength="4096" /></label>
      <label>Instructions<textarea bind:value={instructions} placeholder="Explain the work to perform, the expected output, and any constraints." required rows="4"></textarea></label>
      <label class="checkbox-label"><input type="checkbox" bind:checked={supplyBrief} /> Supply brief now</label>
      {#if supplyBrief}<label>Brief input<textarea bind:value={brief} placeholder="Project details, a question, or data the task should use." rows="4"></textarea></label>{:else}<p class="field-hint">The task requires a brief. It will stay queued until you supply that input in its inspector.</p>{/if}
      <label>Text file input<input type="file" bind:files accept="text/*,.md,.toml,.json,.csv,.rs,.txt" onchange={captureFile} /></label>
      <p class="field-hint">UTF-8 text up to {(session.input_limit_bytes / 1024).toLocaleString()} KiB. The daemon also checks the combined instructions and selected inputs.</p>
      {#if upload}<p class="notice">Captured {upload.name} · {upload.bytes.toLocaleString()} bytes<br /><small>Input digest: <code>{upload.digest}</code></small></p>{/if}
      <details><summary>Project filesystem access</summary><p class="field-hint">For configured process work that needs a native project. The daemon validates the path against your grant.</p><label>Absolute project root<input bind:value={path} placeholder="/home/you/projects/animation" /></label><label class="checkbox-label"><input type="checkbox" bind:checked={pathWrite} /> Allow writes within this root</label></details>
    </fieldset>
    {#if !session.capabilities.length}<p class="notice warning">No permitted capabilities are available. Ask the daemon administrator to configure a capability and your execution allowance.</p>{/if}
    {#if error}<p class="notice error" role="alert">{error}</p>{/if}
    {#if pendingUpload}<button type="button" onclick={retryUpload} disabled={busy}>Retry exact upload</button>{/if}
    {#if pending}
      <p class="notice warning">The submission may have been accepted. Look up its receipt or replay the exact request. Both use request <code>{pending.request_id}</code>.</p>
      <div class="form-actions"><button type="button" class="primary" disabled={busy} onclick={findReceipt}>Find submission receipt</button><button type="button" disabled={busy} onclick={() => submit()}>Replay exact submission</button></div>
    {:else}
      <div class="form-actions"><button class="primary" disabled={busy || !session.capabilities.length || pendingUpload !== null}>{busy ? 'Submitting…' : 'Submit task'}</button><button type="button" class="quiet" onclick={oncancel}>Cancel</button></div>
    {/if}
  </form>
</section>
