<script module lang="ts">
  import type { ProjectCommandRequest, ProjectSettingsDto } from '../lib/protocol.generated';

  /** The exact request belongs to its authenticated connection, including after navigation. */
  export interface ProjectDraft {
    projectId: string | null;
    mode: 'attach' | 'create' | 'settings';
    name: string;
    folder: string;
    folderName: string;
    settings: ProjectSettingsDto;
    reviewedGeneration: number | null;
    dirty: boolean;
    pending: ProjectCommandRequest | null;
    recovery: { operationId: string; folder: string | null } | null;
  }
  export function newProjectDraft(projectId: string | null = null): ProjectDraft {
    return { projectId, mode: projectId ? 'settings' : 'attach', name: '', folder: '', folderName: '', settings: { environment: { kind: 'host' }, allow_changes: false }, reviewedGeneration: null, dirty: false, pending: null, recovery: null };
  }
</script>

<script lang="ts">
  import { onDestroy } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import type { ProjectDto, ProjectReceiptDto, ProjectCommandDto, ProjectDirectoryPageDto, ProjectEnvironmentPageDto, TaskSessionDto } from '../lib/protocol.generated';
  import { DaemonClient, DaemonError } from '../lib/transport';
  import { copy } from '../lib/canvas';

  let { client, session, project, draft = $bindable(), onaccepted, onrefresh, onerror, onback, onmodels }: {
    client: DaemonClient; session: TaskSessionDto; project: ProjectDto | null; draft: ProjectDraft;
    onaccepted: (receipt: ProjectReceiptDto, command: ProjectCommandDto) => Promise<void>; onrefresh: () => Promise<void>;
    onerror: (failure: unknown) => void; onback: () => void; onmodels: () => void;
  } = $props();
  let directories = $state<ProjectDirectoryPageDto | null>(null);
  let environments = $state<ProjectEnvironmentPageDto | null>(null);
  let trail = $state<string[]>([]);
  let error = $state('');
  let notice = $state('');
  let busy = $state(false);
  let loading = $state(false);
  let readGeneration = 0;
  const lifetime = new AbortController();
  onDestroy(() => lifetime.abort());

  $effect(() => {
    if (project && project.id === draft.projectId && !draft.dirty && !draft.pending) {
      draft.name = project.name; draft.folder = project.folder; draft.settings = copy(project.settings);
      draft.reviewedGeneration = project.generation;
    }
  });
  $effect(() => { void loadEnvironments(client); });
  async function loadEnvironments(connection: DaemonClient) {
    try {
      const choices = await connection.request<ProjectEnvironmentPageDto>('/v1/project-environments', undefined, lifetime.signal);
      if (!lifetime.signal.aborted && connection === client) environments = choices;
    }
    catch (failure) { report(failure); }
  }
  function report(failure: unknown) {
    if (lifetime.signal.aborted) return;
    error = failure instanceof Error ? failure.message : 'The daemon request failed.';
    if (failure instanceof DaemonError && (failure.authorizationLost || failure.code === 'daemon_identity_mismatch')) onerror(failure);
  }
  async function browse(parent?: string, cursor?: string) {
    const generation = ++readGeneration;
    loading = true; error = '';
    const query = new SvelteURLSearchParams({ limit: '30' });
    if (parent) query.set('parent', parent);
    if (cursor) query.set('cursor', cursor);
    try {
      const page = await client.request<ProjectDirectoryPageDto>(`/v1/project-directories?${query}`, undefined, lifetime.signal);
      if (generation === readGeneration && !lifetime.signal.aborted) directories = page;
    } catch (failure) { if (generation === readGeneration) report(failure); }
    finally { if (generation === readGeneration) loading = false; }
  }
  function selectFolder(path: string, name: string) {
    draft.folder = path; draft.dirty = true;
    if (draft.mode === 'attach') draft.name = name;
  }
  function openFolder(path: string) {
    if (directories?.parent) trail = [...trail, directories.parent].slice(-32);
    void browse(path);
  }
  async function send(lookup = false) {
    const submittedDraft = draft;
    const request = submittedDraft.pending;
    if (!request || busy) return;
    const actor = session.actor_id;
    const daemon = session.daemon_id;
    const current = () => !lifetime.signal.aborted && draft === submittedDraft && draft.pending === request && session.actor_id === actor && session.daemon_id === daemon;
    busy = true; error = ''; notice = '';
    try {
      const receipt = await client.request<ProjectReceiptDto>(
        lookup ? `/v1/project-receipts/${encodeURIComponent(request.request_id)}` : '/v1/project-commands',
        lookup ? undefined : request, lifetime.signal,
      );
      if (!current()) return;
      if (receipt.request_id !== request.request_id) throw new Error('The reply belongs to a different project request.');
      draft.pending = null;
      const saved = receipt.project;
      if (saved && saved.id === draft.projectId && ['rename', 'settings', 'associate'].includes(request.command.kind)) {
        // Each form saves only its own fields. Preserve the other form's unsent choices.
        if (request.command.kind === 'rename') draft.name = saved.name;
        if (request.command.kind === 'settings') draft.settings = copy(saved.settings);
        draft.reviewedGeneration = saved.generation;
        draft.dirty = draft.name !== saved.name || JSON.stringify(draft.settings) !== JSON.stringify(saved.settings);
      } else draft.dirty = false;
      draft.recovery = receipt.status === 'unresolved' && receipt.operation_id ? { operationId: receipt.operation_id, folder: receipt.operation_folder } : null;
      notice = draft.recovery ? 'Folder creation has an unresolved outcome. The daemon has preserved the folder. Inspect its location, then attach it explicitly or abandon this registration attempt.' : receipt.status === 'abandoned' ? 'Registration attempt abandoned. Files are preserved.' : 'Saved on this daemon.';
      await onaccepted(receipt, request.command);
    } catch (failure) {
      if (!current()) return;
      report(failure);
      if (!lookup && failure instanceof DaemonError && failure.status < 500) {
        draft.pending = null;
        if (failure.status === 409 || failure.status === 403) await onrefresh();
      }
    } finally { if (!lifetime.signal.aborted) busy = false; }
  }
  function command(value: ProjectCommandDto) {
    if (locked || ('project_id' in value && value.project_id !== draft.projectId)) return;
    draft.pending = { request_id: crypto.randomUUID(), command: value };
    void send();
  }
  function create(event: SubmitEvent) {
    event.preventDefault();
    if (!draft.folder) { error = 'Choose a folder on this daemon.'; return; }
    command(draft.mode === 'create'
      ? { kind: 'create', name: draft.name || draft.folderName, parent: draft.folder, folder_name: draft.folderName, settings: copy(draft.settings) }
      : { kind: 'attach', name: draft.name, folder: draft.folder, settings: copy(draft.settings) });
  }
  const locked = $derived(busy || !!draft.pending || (project?.id ?? null) !== draft.projectId || !!project && !project.can_manage);
  const stale = $derived(!!project && draft.reviewedGeneration !== null && draft.reviewedGeneration !== project.generation);
</script>

<section class="panel project-settings" aria-labelledby="project-heading">
  <div class="section-heading"><div><p class="eyebrow">{session.daemon_name}</p><h1 id="project-heading">{project ? 'Project settings' : 'Add project'}</h1></div><button class="quiet" onclick={onback}>Back to work</button></div>
  <p>The folder is on <strong>{session.daemon_name}</strong>. Your editor and Git can keep using it.</p>
  {#if project}
    <dl class="facts"><dt>Folder</dt><dd>{project.folder}</dd><dt>Availability</dt><dd>{project.availability}</dd></dl>
    {#if stale}<p class="notice warning">Project settings changed while you were editing. Your choices are retained. Current access is {project.settings.allow_changes ? 'read and write' : 'read only'} in the {project.settings.environment.kind} environment.</p><button disabled={locked} onclick={() => draft.reviewedGeneration = project!.generation}>Keep my choices after reviewing current settings</button><button disabled={locked} onclick={() => { draft.dirty = false; }}>Use current project settings</button>{/if}
    <form onsubmit={(event) => { event.preventDefault(); command({ kind: 'rename', project_id: project!.id, expected_generation: draft.reviewedGeneration!, name: draft.name }); }}>
      <label>Project name<input bind:value={draft.name} oninput={() => draft.dirty = true} disabled={locked} maxlength="256" /></label>
      <button disabled={locked || stale}>Rename project</button>
    </form>
    <p class="field-hint">Renaming changes the label for all clients. It keeps the folder name and files.</p>
  {:else}
    <div class="form-actions" role="group" aria-label="Project folder choice">
      <button aria-pressed={draft.mode === 'attach'} disabled={locked} onclick={() => { draft.mode = 'attach'; directories = null; }}>Use existing folder</button>
      <button aria-pressed={draft.mode === 'create'} disabled={locked} onclick={() => { draft.mode = 'create'; directories = null; }}>Create folder</button>
    </div>
    <p>{draft.mode === 'create' ? 'Choose an existing parent, then name one new folder. Existing contents are never adopted by creation.' : 'Choose an existing folder. Attaching registers its location and preserves its files.'}</p>
    <button disabled={locked || loading} onclick={() => { trail = []; void browse(); }}>Browse permitted folders</button>
    {#if loading}<p role="status">Reading folders on {session.daemon_name}…</p>{/if}
    {#if directories}
      <div class="folder-chooser" aria-label={`Folders on ${session.daemon_name}`}>
        {#if directories.parent}<p class="folder-location">{directories.parent}</p><div class="form-actions"><button disabled={locked} onclick={() => selectFolder(directories!.parent!, directories!.parent!.split('/').filter(Boolean).at(-1) ?? 'Project')}>Use this {draft.mode === 'create' ? 'parent' : 'folder'}</button><button disabled={locked} onclick={() => { const parent = trail.at(-1); trail = trail.slice(0, -1); void browse(parent); }}>Back to folders</button></div>{/if}
        <ul>{#each directories.items as folder (folder.path)}<li><button disabled={locked} onclick={() => openFolder(folder.path)}>{folder.name}<span aria-hidden="true"> →</span></button><button class="quiet" disabled={locked || draft.mode === 'create' && !folder.can_create} onclick={() => selectFolder(folder.path, folder.name)} aria-label={`Use ${folder.name}`}>Use</button></li>{/each}</ul>
        {#if !directories.items.length}<p class="field-hint">No accessible child folders on this page.</p>{/if}
        {#if directories.next_cursor}<button disabled={locked || loading} onclick={() => browse(directories!.parent ?? undefined, directories!.next_cursor!)}>Next folders</button>{/if}
      </div>
    {/if}
    <form onsubmit={create}>
      {#if draft.folder}<p class="folder-location"><strong>{draft.mode === 'create' ? 'Parent' : 'Folder'}:</strong> {draft.folder}</p>{/if}
      {#if draft.mode === 'create'}<label>New folder name<input bind:value={draft.folderName} oninput={() => { draft.dirty = true; draft.name = draft.folderName; }} disabled={locked} maxlength="255" autocomplete="off" /></label>{/if}
      <label>Project name<input bind:value={draft.name} oninput={() => draft.dirty = true} disabled={locked} maxlength="256" /></label>
      <button class="primary" disabled={locked || !draft.folder || !draft.name.trim() || draft.mode === 'create' && !draft.folderName.trim()}>{busy ? 'Saving…' : draft.mode === 'create' ? 'Create project folder' : 'Attach project'}</button>
    </form>
    <p class="field-hint">New projects begin with read-only host file access. Choose coding tools and allow changes in Project settings afterward.</p>
  {/if}
  {#if project}
    <form onsubmit={(event) => { event.preventDefault(); command({ kind: 'settings', project_id: project!.id, expected_generation: draft.reviewedGeneration!, settings: copy(draft.settings) }); }}>
      <label>Execution environment<select disabled={locked || !environments} value={JSON.stringify(draft.settings.environment)} onchange={(event) => { const selected = environments?.items.find((item) => JSON.stringify(item.environment) === event.currentTarget.value); if (selected) { draft.settings.environment = copy(selected.environment); draft.dirty = true; } }}>
        {#if !environments?.items.some((item) => JSON.stringify(item.environment) === JSON.stringify(draft.settings.environment))}<option value={JSON.stringify(draft.settings.environment)}>Previously selected environment · unavailable</option>{/if}
        {#each environments?.items ?? [] as environment (JSON.stringify(environment.environment))}<option value={JSON.stringify(environment.environment)} disabled={!environment.available}>{environment.label}{environment.available ? '' : ' · unavailable'}</option>{/each}
      </select></label>
      {#each environments?.items.filter((item) => JSON.stringify(item.environment) === JSON.stringify(draft.settings.environment) && item.reason) ?? [] as environment (JSON.stringify(environment.environment))}<p class="notice">{environment.reason}</p>{/each}
      <label class="checkbox-label"><input type="checkbox" checked={draft.settings.allow_changes} disabled={locked} onchange={(event) => { draft.settings.allow_changes = event.currentTarget.checked; draft.dirty = true; }} /> Allow project changes</label>
      <p><strong>Project access:</strong> {draft.settings.allow_changes ? 'Read and write for new tasks, subject to your permission.' : 'Read only. Mutation-capable host tools are excluded.'}</p>
      <p class="field-hint">Settings apply to newly submitted work. Accepted tasks keep their recorded environment and permissions. Host tools are trusted processes; file claims do not sandbox them or lock out your editor.</p>
      <button class="primary" disabled={locked || stale}>Save project settings</button>
    </form>
    <p class="field-hint">Choose the model separately on each task. Inference may run elsewhere while tools use this folder on {session.daemon_name}.</p>
    <button class="quiet" onclick={onmodels}>Models & agents setup</button>
    {#if project.can_manage}<details><summary>Remove registration</summary><p>Only an unused project can be removed. Its folder and files are preserved.</p><button disabled={locked} onclick={() => command({ kind: 'remove', project_id: project!.id, expected_generation: project!.generation })}>Remove unused registration</button></details>{/if}
  {/if}
  {#if error}<p role="alert" class="notice error">{error} Your choices are retained.</p>{/if}
  {#if draft.dirty && !draft.pending}<button class="quiet" disabled={busy} onclick={() => { Object.assign(draft, newProjectDraft(draft.projectId)); error = ''; }}>Discard project draft</button>{/if}
  {#if draft.pending && !busy}<p class="notice warning">The reply is unconfirmed. Recover this exact request before submitting another.</p><div class="form-actions"><button onclick={() => send(true)}>Find project receipt</button><button onclick={() => send()}>Replay exact project request</button></div>{/if}
  {#if notice}<p role="status" class="notice">{notice}</p>{/if}
  {#if draft.recovery}<p class="notice warning">Unresolved folder creation{draft.recovery.folder ? ` at ${draft.recovery.folder}` : ''}. Inspect that folder on {session.daemon_name} before attaching it. Abandoning preserves any files.</p><div class="form-actions"><button disabled={locked} onclick={() => command({ kind: 'reconcile', operation_id: draft.recovery!.operationId })}>Attach inspected folder</button><button disabled={locked} onclick={() => command({ kind: 'abandon', operation_id: draft.recovery!.operationId })}>Abandon registration attempt</button></div>{/if}
</section>
