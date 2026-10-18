<script lang="ts">
  import { onDestroy, onMount, tick, untrack } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import ConnectionForm from './components/ConnectionForm.svelte';
  import DaemonSettings, { type RenameDraft } from './components/DaemonSettings.svelte';
  import TaskComposer from './components/TaskComposer.svelte';
  import WorkflowView from './components/WorkflowView.svelte';
  import { endpointUrl, loadProfiles, maxConnections, saveProfiles } from './lib/connections';
  import type { ConnectionProfile } from './lib/connections';
  import type { RenameDaemonReceipt, TaskCapability, TaskSessionDto, WorkflowPageDto, WorkflowReadDto, WorkflowReceipt, WorkflowSummaryDto } from './lib/protocol.generated';
  import { createTaskDraft, type TaskDraft, type TaskDraftTarget } from './lib/task-composer';
  import { DaemonClient, DaemonError } from './lib/transport';
  import { timeLabel } from './lib/presentation';

  interface Connection {
    profile: ConnectionProfile;
    client: DaemonClient;
    session: TaskSessionDto;
    rename: RenameDraft;
    workflows: WorkflowSummaryDto[];
    nextCursor: string | null;
    pageCursor: string | undefined;
    truncated: boolean;
    search: string;
    appliedSearch: string;
    selected: WorkflowReadDto | null;
    selectedId: string;
    selectionGeneration: number;
    sessionGeneration: number;
    recent: WorkflowSummaryDto[];
    drafts: TaskDraft[];
    visibleDraftId: string;
    preferredCapability?: TaskCapability;
    view: 'work' | 'settings';
    loading: boolean;
    taskCursor: string | undefined;
    error: string;
    observation: string;
    stream: AbortController;
    cursor: string | undefined;
    refreshGeneration: number;
    busy: boolean;
  }

  let profiles = $state(loadProfiles());
  let connections = $state<Connection[]>([]);
  let activeId = $state('');
  let connectionForm = $state(false);
  let reconnectProfile = $state<ConnectionProfile | undefined>();
  let globalError = $state('');
  let navigationOpen = $state(false);
  let savedConnectionsOpen = $state(untrack(() => profiles.length > 0));
  let navigationButton: HTMLButtonElement | undefined;
  let bootstrap: DaemonClient | undefined;
  let bootstrapGeneration = 0;
  let active = $derived(connections.find((connection) => connection.profile.id === activeId));
  let visibleDraft = $derived(active?.drafts.find((draft) => draft.id === active.visibleDraftId));
  const maxDrafts = 8;
  let workflowView: { canNavigate: () => boolean } | undefined = $state();

  function mayLeaveWork() {
    return active?.view !== 'work' || connectionForm || !active?.selected || !workflowView || workflowView.canNavigate();
  }

  interface Location { connection: string; daemon: string; workflow: string; view: 'work' | 'settings'; draft: string }
  function rememberLocation(replace = false) {
    const location: Location | null = active ? { connection: active.profile.id, daemon: active.session.daemon_id, workflow: active.selectedId, view: active.view, draft: active.visibleDraftId } : null;
    window.history[replace ? 'replaceState' : 'pushState']({ milkdrift: location }, '');
  }

  function restoreLocation(event: PopStateEvent) {
    const location = event.state?.milkdrift as Location | undefined;
    if (!mayLeaveWork()) { rememberLocation(true); return; }
    if (!location || typeof location.connection !== 'string') { closeConnectionForm(); activeId = ''; return; }
    closeConnectionForm();
    const connection = connections.find((item) => item.profile.id === location.connection && item.session.daemon_id === location.daemon);
    if (!connection) {
      const profile = profiles.find((item) => item.id === location.connection && item.daemonId === location.daemon);
      activeId = '';
      if (profile) { reconnectProfile = profile; connectionForm = true; }
      return;
    }
    activeId = connection.profile.id;
    connection.view = location.view === 'settings' ? 'settings' : 'work';
    connection.visibleDraftId = connection.drafts.some((draft) => draft.id === location.draft) ? location.draft : '';
    void selectWorkflow(connection, location.workflow || '', undefined, false, false);
    closeNavigation();
  }
  onMount(() => {
    window.addEventListener('popstate', restoreLocation);
    return () => window.removeEventListener('popstate', restoreLocation);
  });

  function closeNavigation() {
    if (navigationOpen) {
      navigationOpen = false;
      navigationButton?.focus();
    }
  }

  function showWorkflows() {
    if (!mayLeaveWork()) return;
    closeConnectionForm();
    if (active) { active.view = 'work'; active.visibleDraftId = ''; void selectWorkflow(active, ''); }
    closeNavigation();
  }

  function showSettings() {
    if (!mayLeaveWork()) return;
    closeConnectionForm();
    if (active) { active.view = 'settings'; rememberLocation(); }
    closeNavigation();
  }

  function fail(connection: Connection, failure: unknown) {
    if (connection.stream.signal.aborted) return;
    if (failure instanceof Error && failure.name === 'AbortError') return;
    const message = failure instanceof Error ? failure.message : 'The daemon request failed.';
    if (failure instanceof DaemonError && (failure.authorizationLost || failure.code === 'daemon_identity_mismatch')) {
      disconnect(connection);
      globalError = failure.code === 'daemon_identity_mismatch'
        ? `${connection.profile.cachedName}: This address now connects to a different daemon. Forget this saved connection, check the address, then connect afresh.`
        : `${connection.profile.cachedName}: ${message} Authenticate again to read permitted work.`;
    } else connection.error = message;
  }

  function disconnect(connection: Connection) {
    connection.refreshGeneration++;
    connection.stream.abort();
    connection.client.disconnect();
    disposeWork(connection);
    connection.workflows = [];
    connection.selected = null;
    connection.search = '';
    connection.appliedSearch = '';
    connection.nextCursor = null;
    connection.pageCursor = undefined;
    connection.cursor = undefined;
    connection.taskCursor = undefined;
    connections = connections.filter((item) => item.profile.id !== connection.profile.id);
    if (activeId === connection.profile.id) activeId = '';
  }

  function disposeWork(connection: Connection) {
    connection.selectionGeneration++;
    for (const draft of connection.drafts) draft.disposed = true;
    connection.drafts = [];
    connection.visibleDraftId = '';
    connection.preferredCapability = undefined;
    connection.recent = [];
    connection.workflows = [];
    connection.selected = null;
    connection.selectedId = '';
    connection.busy = false;
  }

  function cacheName(connection: Connection, name: string) {
    if (connection.profile.cachedName === name) return;
    connection.profile.cachedName = name;
    profiles = profiles.map((profile) => profile.id === connection.profile.id ? { ...profile, cachedName: name } : profile);
    try { saveProfiles(profiles); }
    catch { globalError = 'The current daemon name is shown, but this browser could not save its connection cache. Reconnect to read the latest name next time.'; }
  }

  function renamed(connection: Connection, receipt: RenameDaemonReceipt) {
    if (connection.stream.signal.aborted || receipt.daemon_id !== connection.session.daemon_id || receipt.generation < connection.session.daemon_name_generation) return;
    connection.session.daemon_name = receipt.name;
    connection.session.daemon_name_generation = receipt.generation;
    cacheName(connection, receipt.name);
  }

  async function refreshSession(connection: Connection) {
    const generation = ++connection.sessionGeneration;
    try {
      const session = await connection.client.request<TaskSessionDto>('/v1/session');
      if (connection.stream.signal.aborted || generation !== connection.sessionGeneration) return;
      if (session.daemon_id !== connection.profile.daemonId) {
        fail(connection, new DaemonError('daemon_identity_mismatch', 'This address now connects to a different daemon.', 409));
        return;
      }
      if (session.actor_id !== connection.session.actor_id) {
        connection.refreshGeneration++;
        connection.loading = false;
        disposeWork(connection);
      }
      const name = session.daemon_name_generation >= connection.session.daemon_name_generation
        ? session : connection.session;
      connection.session = { ...session, daemon_name: name.daemon_name, daemon_name_generation: name.daemon_name_generation };
      cacheName(connection, connection.session.daemon_name);
    } catch (failure) { fail(connection, failure); }
  }

  async function refreshObserved(connection: Connection) {
    // Metadata remains observable even when workflow discovery is slow or unavailable.
    await Promise.all([refreshSession(connection), refresh(connection)]);
  }

  async function refresh(connection: Connection, nextPage = false) {
    const generation = ++connection.refreshGeneration;
    connection.loading = true;
    const pageCursor = nextPage ? connection.nextCursor ?? undefined : connection.pageCursor;
    const query = new SvelteURLSearchParams({ limit: '50', search: connection.appliedSearch });
    if (pageCursor) query.set('cursor', pageCursor);
    try {
      const page = await connection.client.request<WorkflowPageDto>(`/v1/workflows?${query}`);
      if (generation !== connection.refreshGeneration || connection.stream.signal.aborted) return;
      connection.workflows = page.items;
      connection.pageCursor = pageCursor;
      connection.nextCursor = page.next_cursor;
      connection.truncated = page.truncated;
      if (!pageCursor && !connection.appliedSearch) updateRecent(connection, page.items);
      else void refreshRecent(connection, generation);
      connection.error = '';
      if (connection.selectedId) await readSelection(connection);
    } catch (failure) {
      if (generation !== connection.refreshGeneration || connection.stream.signal.aborted) return;
      if (failure instanceof DaemonError && failure.status === 403) {
        disposeWork(connection);
        connection.error = failure.message;
        return;
      }
      if (failure instanceof DaemonError && (failure.status === 410 || failure.code === 'expired_cursor')) {
        connection.nextCursor = null;
        connection.pageCursor = undefined;
        connection.cursor = undefined;
        connection.taskCursor = undefined;
        if (pageCursor) await refresh(connection);
        else fail(connection, failure);
      } else fail(connection, failure);
    } finally { if (generation === connection.refreshGeneration) connection.loading = false; }
  }

  async function refreshRecent(connection: Connection, generation: number) {
    try {
      const page = await connection.client.request<WorkflowPageDto>('/v1/workflows?limit=8&search=');
      if (generation === connection.refreshGeneration && !connection.stream.signal.aborted) updateRecent(connection, page.items);
    } catch (failure) { if (generation === connection.refreshGeneration) fail(connection, failure); }
  }

  function updateRecent(connection: Connection, items: WorkflowSummaryDto[]) {
    // Discovery has its own stable page order. Keep recently visited work ahead of page suggestions.
    const updated = connection.recent.map((item) => items.find((candidate) => candidate.id === item.id) ?? item);
    connection.recent = [...updated, ...items.filter((item) => !updated.some((known) => known.id === item.id))].slice(0, 8);
  }

  async function observe(connection: Connection) {
    const signal = connection.stream.signal;
    while (!signal.aborted) {
      let resync = false;
      try {
        connection.observation = 'Connecting live updates';
        await connection.client.stream('/v1/workflows/stream', async (event) => {
          if (signal.aborted) return;
          connection.observation = 'Live updates';
          if (event.event === 'resync_required') {
            connection.cursor = undefined;
            connection.taskCursor = undefined;
            connection.nextCursor = null;
            connection.pageCursor = undefined;
            resync = true;
            await refreshObserved(connection);
          } else if (event.event === 'workflow_changed') {
            if (event.id) connection.cursor = event.id;
            await refreshObserved(connection);
          } else if (event.event === 'forbidden') {
            fail(connection, new DaemonError('unauthorized', 'Observation authority changed.', 403));
          }
        }, signal, connection.cursor);
      } catch (failure) {
        if (signal.aborted) return;
        if (failure instanceof DaemonError && (failure.status === 410 || failure.code === 'expired_cursor')) {
          connection.cursor = undefined;
          connection.pageCursor = undefined;
          connection.taskCursor = undefined;
          resync = true;
          await refreshObserved(connection);
        } else fail(connection, failure);
      }
      if (signal.aborted) return;
      connection.observation = resync ? 'Refreshing snapshot' : 'Reconnecting live updates';
      await new Promise<void>((resolve) => {
        const timer = window.setTimeout(finish, 1500);
        function finish() { window.clearTimeout(timer); signal.removeEventListener('abort', finish); resolve(); }
        signal.addEventListener('abort', finish, { once: true });
      });
    }
  }

  function cancelBootstrap() {
    bootstrapGeneration++;
    bootstrap?.disconnect();
    bootstrap = undefined;
  }

  function closeConnectionForm() { cancelBootstrap(); connectionForm = false; }

  async function connect(details: { endpoint: string; token: string }) {
    const previous = reconnectProfile;
    if (!previous && profiles.length >= maxConnections) throw new Error(`At most ${maxConnections} connections can be saved. Forget a connection before adding another.`);
    cancelBootstrap();
    const generation = bootstrapGeneration;
    const endpoint = endpointUrl(details.endpoint);
    const client = new DaemonClient(endpoint, details.token);
    bootstrap = client;
    if (previous) {
      const live = connections.find((item) => item.profile.id === previous.id);
      if (live) disconnect(live);
    }
    try {
      const session = await client.request<TaskSessionDto>('/v1/session');
      if (generation !== bootstrapGeneration) return;
      if (previous && session.daemon_id !== previous.daemonId) throw new Error('This address now connects to a different daemon. Forget this saved connection, check the address, then connect afresh. Your previous work has not been transferred.');
      client.bindIdentity(session.daemon_id);
      const profile = { id: previous?.id ?? crypto.randomUUID(), cachedName: session.daemon_name, endpoint, daemonId: session.daemon_id };
      profiles = [...profiles.filter((item) => item.id !== profile.id), profile];
      try { saveProfiles(profiles); } catch { globalError = 'This browser could not save the connection cache. The authenticated connection and current daemon name still work for this session.'; }
      const connection: Connection = {
        profile, client, session, rename: { name: session.daemon_name, dirty: false, pending: null }, workflows: [], nextCursor: null, pageCursor: undefined, truncated: false, search: '', appliedSearch: '', selected: null, taskCursor: undefined,
        selectedId: '', selectionGeneration: 0, sessionGeneration: 0, recent: [], drafts: [], visibleDraftId: '', view: 'work', loading: true,
        error: '', observation: 'Connecting live updates', stream: new AbortController(), cursor: undefined, refreshGeneration: 0, busy: false,
      };
      connections = [...connections, connection];
      activeId = profile.id;
      connectionForm = false;
      savedConnectionsOpen = false;
      bootstrap = undefined;
      // Read and update through the reactive connection proxy after insertion.
      const installed = connections.find((item) => item.profile.id === profile.id)!;
      const restored = window.history.state?.milkdrift as Location | undefined;
      if (restored?.connection === profile.id && restored.daemon === session.daemon_id) {
        installed.view = restored.view;
        void selectWorkflow(installed, restored.workflow, undefined, false);
      }
      rememberLocation();
      void observe(installed);
      await refresh(installed);
    } catch (failure) { client.disconnect(); throw failure; }
  }

  function choose(profile: ConnectionProfile) {
    if (profile.id !== activeId && !mayLeaveWork()) return;
    cancelBootstrap();
    globalError = '';
    const live = connections.find((item) => item.profile.id === profile.id);
    if (live) { activeId = profile.id; connectionForm = false; rememberLocation(); }
    else { reconnectProfile = profile; connectionForm = true; }
    closeNavigation();
  }

  function addConnection() { if (!mayLeaveWork()) return; cancelBootstrap(); reconnectProfile = undefined; connectionForm = true; globalError = ''; closeNavigation(); }

  function forget(profile: ConnectionProfile) {
    if (reconnectProfile?.id === profile.id) cancelBootstrap();
    const live = connections.find((connection) => connection.profile.id === profile.id);
    if (live) disconnect(live);
    profiles = profiles.filter((item) => item.id !== profile.id);
    if (reconnectProfile?.id === profile.id) { reconnectProfile = undefined; connectionForm = false; }
    try { saveProfiles(profiles); } catch { globalError = 'This browser could not remove the saved connection. It may reappear after reloading; clear this site’s saved data to forget it permanently.'; }
  }

  async function readSelection(connection: Connection) {
    const id = connection.selectedId;
    const generation = ++connection.selectionGeneration;
    if (!id) { connection.selected = null; connection.busy = false; return; }
    connection.busy = true;
    try {
      const suffix = connection.taskCursor ? `?cursor=${encodeURIComponent(connection.taskCursor)}` : '';
      const selected = await connection.client.request<WorkflowReadDto>(`/v1/workflows/${encodeURIComponent(id)}${suffix}`);
      if (connection.stream.signal.aborted || generation !== connection.selectionGeneration || connection.selectedId !== id) return;
      const visiting = connection.selected?.id !== id;
      connection.selected = selected;
      const summary: WorkflowSummaryDto = { id: selected.id, title: selected.title, description: selected.description, generation: selected.generation, state: selected.state, created_at: selected.created_at, updated_at: selected.updated_at, task_count: selected.task_count };
      connection.recent = visiting
        ? [summary, ...connection.recent.filter((item) => item.id !== id)].slice(0, 8)
        : connection.recent.map((item) => item.id === id ? summary : item);
    }
    catch (failure) { if (generation === connection.selectionGeneration) fail(connection, failure); }
    finally { if (generation === connection.selectionGeneration) connection.busy = false; }
  }

  async function selectWorkflow(connection: Connection, id: string, cursor?: string, history = true, resumeDraft = true) {
    if (connection === active && connection.selectedId !== id && !mayLeaveWork()) return;
    closeConnectionForm();
    if (connection.selectedId !== id) connection.selected = null;
    connection.selectedId = id;
    connection.taskCursor = cursor;
    if (history) connection.view = 'work';
    if (resumeDraft) connection.visibleDraftId = connection.drafts.find((draft) => draft.target.kind === 'existing' && draft.target.workflowId === id)?.id ?? '';
    if (history) rememberLocation();
    closeNavigation();
    await readSelection(connection);
  }

  function search(connection: Connection) {
    connection.pageCursor = undefined;
    connection.appliedSearch = connection.search;
    void refresh(connection);
  }

  function openComposer(connection: Connection, target: TaskDraftTarget) {
    if (!mayLeaveWork()) return;
    let draft = connection.drafts.find((item) => item.target.kind === target.kind && (target.kind === 'new' || (item.target.kind === 'existing' && item.target.workflowId === target.workflowId)));
    if (!draft) {
      if (connection.drafts.length >= maxDrafts) { connection.error = `This connection holds ${maxDrafts} drafts. Open and discard one before starting another; your existing drafts are intact.`; return; }
      draft = createTaskDraft(connection.session, target, connection.preferredCapability);
      connection.drafts = [...connection.drafts, draft];
    }
    connection.visibleDraftId = draft.id;
    connection.view = 'work';
    connection.error = '';
    closeConnectionForm();
    closeNavigation();
    rememberLocation();
  }

  async function closeComposer(connection: Connection, draft: TaskDraft, discard = false) {
    if (discard) {
      if (draft.busy || draft.pending || draft.pendingUpload) return;
      draft.disposed = true;
      connection.drafts = connection.drafts.filter((item) => item.id !== draft.id);
    }
    if (connection.visibleDraftId === draft.id) connection.visibleDraftId = '';
    rememberLocation(true);
    await tick();
    if (active === connection && connection.view === 'work') {
      if (draft.target.kind === 'new' && window.innerWidth <= 760 && !navigationOpen) navigationButton?.focus();
      else document.querySelector<HTMLButtonElement>(draft.target.kind === 'new' ? '.new-workflow' : '[data-add-task]')?.focus();
    }
  }

  async function resumeDraft(connection: Connection, draft: TaskDraft) {
    if (draft.disposed || !mayLeaveWork()) return;
    const read = draft.target.kind === 'existing' ? selectWorkflow(connection, draft.target.workflowId, undefined, false, false) : Promise.resolve();
    connection.view = 'work';
    connection.visibleDraftId = draft.id;
    closeNavigation();
    rememberLocation();
    await read;
  }

  function composerError(connection: Connection, failure: unknown) {
    if (failure instanceof DaemonError && (failure.authorizationLost || failure.code === 'daemon_identity_mismatch')) fail(connection, failure);
  }

  function reviewed(connection: Connection, draft: TaskDraft, workflow: WorkflowReadDto) {
    if (draft.disposed || connection.stream.signal.aborted || connection.selectedId !== workflow.id) return;
    if (connection.selected && connection.selected.generation > workflow.generation) return;
    connection.selectionGeneration++;
    connection.busy = false;
    connection.selected = workflow;
  }

  async function accepted(connection: Connection, draft: TaskDraft, receipt: WorkflowReceipt) {
    if (connection.stream.signal.aborted || draft.disposed) return;
    const stillHere = activeId === connection.profile.id && connection.visibleDraftId === draft.id && connection.view === 'work' && !connectionForm;
    const restoreFocus = stillHere && (document.activeElement === document.body || !!document.activeElement?.closest('.task-composer'));
    connection.drafts = connection.drafts.filter((item) => item.id !== draft.id);
    if (connection.visibleDraftId === draft.id) connection.visibleDraftId = '';
    if (stillHere) {
      await selectWorkflow(connection, receipt.workflow_id);
      await tick();
      if (restoreFocus && document.activeElement === document.body && active === connection && connection.selected?.id === receipt.workflow_id && connection.view === 'work' && !connection.visibleDraftId && !connectionForm) document.querySelector<HTMLButtonElement>('[data-add-task]')?.focus();
    }
    await refresh(connection);
  }

  onDestroy(() => { cancelBootstrap(); for (const connection of connections) { connection.stream.abort(); connection.client.disconnect(); } });
</script>

<svelte:head><meta name="description" content="Submit work to your Milkdrift daemon, follow its progress, and return to retained results." /></svelte:head>
<a class="skip-link" href="#main">Skip to workspace</a>
<div class="workspace">
  <header class="app-header">
    <img class="mobile-logo" src="/milkdrift-logo.svg" alt="" width="34" height="32" />
    <button class="navigation-toggle" bind:this={navigationButton} aria-label="Workspace navigation" aria-expanded={navigationOpen} aria-controls="daemon-navigation" onclick={() => navigationOpen = !navigationOpen}><span aria-hidden="true">☰</span> Navigate</button>
    <div class="workspace-context"><span class="eyebrow">{active ? active.session.daemon_name : 'Milkdrift workspace'}</span><span class="context-title">{connectionForm ? 'Connection' : active?.view === 'settings' ? 'Daemon settings' : active?.selected?.title ?? (active?.selectedId ? 'Loading workflow…' : 'All workflows')}</span></div>
    <div class="header-status"><span class="status-dot" class:connected={!!active} aria-hidden="true"></span>{active ? 'Connected' : 'Not connected'}</div>
  </header>
  <aside id="daemon-navigation" class="sidebar" class:navigation-open={navigationOpen} aria-label="Workspace navigation">
    <div class="brand-lockup"><img class="brand-logo" src="/milkdrift-logo.svg" alt="" width="46" height="44" /><span class="brand">Milkdrift</span></div>
    <div class="sidebar-heading"><p class="eyebrow">Daemon</p><button class="quiet" onclick={addConnection} aria-label="Add daemon connection">+</button></div>
    {#if profiles.length}<label class="daemon-switcher"><span class="visually-hidden">Daemon</span><select value={activeId} onchange={(event) => { const profile = profiles.find((item) => item.id === event.currentTarget.value); if (profile) choose(profile); }}><option value="" disabled>Choose a daemon</option>{#each profiles as profile (profile.id)}<option value={profile.id}>{profile.cachedName}{connections.some((item) => item.profile.id === profile.id) ? '' : ' · Reconnect'}</option>{/each}</select></label>{/if}
    <details class="saved-connections" bind:open={savedConnectionsOpen}><summary>Saved connections</summary><ul class="connection-list">{#each profiles as profile (profile.id)}{@const connected = connections.some((item) => item.profile.id === profile.id)}<li><button class:active={activeId === profile.id} onclick={() => choose(profile)} aria-pressed={activeId === profile.id}><span class="status-dot" class:connected aria-hidden="true"></span><span>{profile.cachedName}<small>{profile.endpoint}</small><small>{connected ? 'Connected' : 'Cached name · Authenticate to reconnect'}</small></span></button><button class="quiet forget-connection" onclick={() => forget(profile)} aria-label={`Forget ${profile.cachedName} connection`}>Forget</button></li>{/each}</ul></details>
    {#if !profiles.length}<p class="sidebar-footer">Add your first daemon to start a task.</p>{/if}
    {#if active}
      <button class="primary new-workflow" aria-label="New workflow" onclick={() => openComposer(active!, { kind: 'new' })}>+ New workflow</button>
      <nav class="recent-workflows" aria-label="Recent workflows">
        <p class="eyebrow">Current & recent</p>
        {#if active.selected && !active.recent.some((item) => item.id === active!.selectedId)}<button class="active" aria-current="page" aria-label={active.selected.title} onclick={() => selectWorkflow(active!, active!.selectedId)}>{active.selected.title}<small>Current workflow</small></button>{/if}
        {#each active.recent as workflow (workflow.id)}<button aria-label={workflow.title} class:active={active.selectedId === workflow.id && active.view === 'work'} aria-current={active.selectedId === workflow.id && active.view === 'work' ? 'page' : undefined} onclick={() => selectWorkflow(active!, workflow.id)}>{workflow.title}<small>{workflow.task_count} {workflow.task_count === 1 ? 'task' : 'tasks'} · {workflow.state}</small></button>{/each}
        {#if active.loading && !active.recent.length}<p class="field-hint">Loading workflows…</p>{/if}
      </nav>
      <nav class="workspace-navigation" aria-label="Workspace">
        <button class:active={active.view === 'work' && !active.selectedId} onclick={showWorkflows}><span aria-hidden="true">▦</span> All workflows</button>
        <button class:active={active.view === 'settings'} onclick={showSettings}><span aria-hidden="true">⚙</span> Daemon settings</button>
      </nav>
      {#if active.drafts.length}<details class="workspace-drafts"><summary>Drafts ({active.drafts.length}/{maxDrafts})</summary><p class="field-hint">Up to {maxDrafts} drafts per connection, held only in this browser session. Open a draft to resume or discard it.</p>{#each active.drafts as draft (draft.id)}<button onclick={() => resumeDraft(active!, draft)}>{draft.definition.title || 'Untitled task'}<small>{draft.target.kind === 'new' ? 'New workflow' : draft.target.title}{draft.pending ? ' · Reply unconfirmed' : draft.busy ? ' · Sending' : ''}</small></button>{/each}</details>{/if}
      <div class="connection-detail"><p class="eyebrow">Connected daemon</p><strong>{active.session.daemon_name}</strong><p class="connection-address">{active.profile.endpoint}</p><p aria-live="polite"><span class="status-dot connected" aria-hidden="true"></span> {active.observation}</p><button class="quiet" onclick={() => { disconnect(active!); closeNavigation(); }}>Disconnect</button></div>
    {/if}
    <p class="sidebar-footer">Closing this client leaves your daemon running.</p>
  </aside>
  <main id="main">
    {#if globalError}<p class="notice error" role="alert">{globalError}</p>{/if}
    {#if connectionForm}
      {#key reconnectProfile?.id}<ConnectionForm profile={reconnectProfile} onconnect={connect} oncancel={closeConnectionForm} />{/key}
    {:else if active}
      {#if active.error}<p class="notice error" role="alert">{active.error}</p>{/if}
      {#key active.profile.id}
        {#if active.view === 'settings'}
          <DaemonSettings client={active.client} session={active.session} bind:draft={active.rename} onrenamed={renamed.bind(null, active)} onrefresh={refreshSession.bind(null, active)} onerror={fail.bind(null, active)} onback={() => { active!.view = 'work'; rememberLocation(); }} />
        {:else}
          <div class="work-and-composer" class:composer-visible={!!visibleDraft}>
          <div class="work-content">
          {#if active.selectedId}
            {#if active.selected}
              <div class="workspace-actions"><button data-add-task class="primary" onclick={() => openComposer(active!, { kind: 'existing', workflowId: active!.selected!.id, title: active!.selected!.title, generation: active!.selected!.generation })}>+ Add task</button></div>
              {#key active.selected.id}<WorkflowView bind:this={workflowView} workflow={active.selected} client={active.client} capabilities={active.session.capabilities} onrefresh={refresh.bind(null, active)} onpage={(cursor) => selectWorkflow(active!, active!.selectedId, cursor, false, false)} onback={showWorkflows} onerror={fail.bind(null, active)} />{/key}
            {:else}<div class="panel" role="status">{active.busy ? 'Loading workflow…' : 'This workflow could not be loaded.'}<button onclick={() => readSelection(active!)}>Retry workflow</button></div>{/if}
          {:else}
          <div class="page-heading"><div><p class="eyebrow">Workspace</p><h1>Your workflows</h1><p>Work in progress and retained results on {active.session.daemon_name}.</p></div></div>
          <form class="toolbar" onsubmit={(event) => { event.preventDefault(); search(active!); }}><label><span class="visually-hidden">Search workflows and retained results</span><input type="search" bind:value={active.search} placeholder="Search workflows and retained results" /></label><button>Search</button><button type="button" onclick={() => refresh(active!)}>Refresh</button></form>
          {#if active.workflows.length}
            <ul class="workflow-list">{#each active.workflows as workflow (workflow.id)}<li><button class="workflow-card" onclick={() => selectWorkflow(active!, workflow.id)}><span><h3>{workflow.title}</h3><p>{workflow.description}</p><small>{workflow.task_count} {workflow.task_count === 1 ? 'task' : 'tasks'} · Updated {timeLabel(workflow.updated_at)}</small></span><span><span class="badge {workflow.state}">{workflow.state}</span></span></button></li>{/each}</ul>
          {:else if active.loading}<p role="status">Loading workflows…</p>
          {:else if !active.error}
            <div class="panel empty"><div class="empty-symbol" aria-hidden="true">↗</div><h2>{active.appliedSearch ? 'No matching workflows' : 'Your first workflow starts with a task'}</h2><p>{active.appliedSearch ? 'The daemon returned no accessible workflows for this search. Try a different title or result phrase.' : 'Choose New workflow, describe the work, and follow its progress here. Add more tasks whenever you need them.'}</p><button class="primary" onclick={() => openComposer(active!, { kind: 'new' })}>Create a workflow</button></div>
          {/if}
          {#if active.truncated}<p class="notice warning">The daemon’s bounded search window was exhausted. Refine the search to inspect more specific work.</p>{/if}
          {#if active.pageCursor || active.nextCursor}<div class="form-actions">{#if active.pageCursor}<button onclick={() => { active!.pageCursor = undefined; void refresh(active!); }}>First page</button>{/if}{#if active.nextCursor}<button onclick={() => refresh(active!, true)}>Next page</button>{/if}</div>{/if}
          {/if}
          </div>
          {#if visibleDraft}{@const connection = active}{@const draft = visibleDraft}<div class="composer-dock">{#key draft.id}<TaskComposer client={connection.client} session={connection.session} {draft} onaccepted={accepted.bind(null, connection, draft)} onreviewed={reviewed.bind(null, connection, draft)} onclose={() => closeComposer(connection, draft)} ondiscard={() => closeComposer(connection, draft, true)} onerror={composerError.bind(null, connection)} oncapabilitychoice={(capability) => connection.preferredCapability = capability} />{/key}</div>{/if}
          </div>
        {/if}
      {/key}
    {:else}
      <div class="page-heading"><div><p class="eyebrow">Your workspace</p><h1>Work continues here.</h1></div></div>
      <div class="panel empty"><div class="empty-symbol" aria-hidden="true">↗</div><h2>Connect to your work</h2><p>Choose a saved daemon or add a connection. Sign in with its scoped credential to discover your tasks and retained results.</p><button class="primary" onclick={addConnection}>Connect a daemon</button></div>
    {/if}
  </main>
</div>
