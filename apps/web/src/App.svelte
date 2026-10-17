<script lang="ts">
  import { onDestroy } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import ConnectionForm from './components/ConnectionForm.svelte';
  import DaemonSettings, { type RenameDraft } from './components/DaemonSettings.svelte';
  import TaskComposer from './components/TaskComposer.svelte';
  import WorkflowView from './components/WorkflowView.svelte';
  import { endpointUrl, loadProfiles, maxConnections, saveProfiles } from './lib/connections';
  import type { ConnectionProfile } from './lib/connections';
  import type { RenameDaemonReceipt, TaskSessionDto, WorkflowPageDto, WorkflowReadDto, WorkflowReceipt, WorkflowSummaryDto } from './lib/protocol.generated';
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
  let composing = $state(false);
  let settings = $state(false);
  let globalError = $state('');
  let navigationOpen = $state(false);
  let navigationButton: HTMLButtonElement | undefined;
  let bootstrap: DaemonClient | undefined;
  let bootstrapGeneration = 0;
  let active = $derived(connections.find((connection) => connection.profile.id === activeId));

  function closeNavigation() {
    if (navigationOpen) {
      navigationOpen = false;
      navigationButton?.focus();
    }
  }

  function showWorkflows() {
    closeConnectionForm();
    composing = false;
    settings = false;
    if (active) active.selected = null;
    closeNavigation();
  }

  function showSettings() {
    closeConnectionForm();
    settings = true;
    composing = false;
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
    connection.workflows = [];
    connection.selected = null;
    connection.search = '';
    connection.appliedSearch = '';
    connection.nextCursor = null;
    connection.pageCursor = undefined;
    connection.cursor = undefined;
    connection.taskCursor = undefined;
    connections = connections.filter((item) => item.profile.id !== connection.profile.id);
    if (activeId === connection.profile.id) { activeId = ''; composing = false; settings = false; }
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
    try {
      const session = await connection.client.request<TaskSessionDto>('/v1/session');
      if (connection.stream.signal.aborted) return;
      if (session.daemon_id !== connection.profile.daemonId) {
        fail(connection, new DaemonError('daemon_identity_mismatch', 'This address now connects to a different daemon.', 409));
        return;
      }
      if (session.daemon_name_generation >= connection.session.daemon_name_generation) {
        connection.session = session;
        cacheName(connection, session.daemon_name);
      }
    } catch (failure) { fail(connection, failure); }
  }

  async function refreshObserved(connection: Connection) {
    // Metadata remains observable even when workflow discovery is slow or unavailable.
    await Promise.all([refreshSession(connection), refresh(connection)]);
  }

  async function refresh(connection: Connection, nextPage = false) {
    const generation = ++connection.refreshGeneration;
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
      connection.error = '';
      if (connection.selected) {
        const selectedId = connection.selected.id;
        const suffix = connection.taskCursor ? `?cursor=${encodeURIComponent(connection.taskCursor)}` : '';
        const snapshot = await connection.client.request<WorkflowReadDto>(`/v1/workflows/${encodeURIComponent(selectedId)}${suffix}`);
        if (generation === connection.refreshGeneration && !connection.stream.signal.aborted && connection.selected?.id === selectedId) connection.selected = snapshot;
      }
    } catch (failure) {
      if (generation !== connection.refreshGeneration || connection.stream.signal.aborted) return;
      if (failure instanceof DaemonError && failure.status === 403) {
        connection.workflows = [];
        connection.selected = null;
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
    }
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
        error: '', observation: 'Connecting live updates', stream: new AbortController(), cursor: undefined, refreshGeneration: 0, busy: false,
      };
      connections = [...connections, connection];
      activeId = profile.id;
      connectionForm = false;
      composing = false;
      settings = false;
      bootstrap = undefined;
      // Read and update through the reactive connection proxy after insertion.
      const installed = connections.find((item) => item.profile.id === profile.id)!;
      void observe(installed);
      await refresh(installed);
    } catch (failure) { client.disconnect(); throw failure; }
  }

  function choose(profile: ConnectionProfile) {
    cancelBootstrap();
    globalError = '';
    composing = false;
    settings = false;
    const live = connections.find((item) => item.profile.id === profile.id);
    if (live) { activeId = profile.id; connectionForm = false; }
    else { reconnectProfile = profile; connectionForm = true; }
    closeNavigation();
  }

  function addConnection() { cancelBootstrap(); reconnectProfile = undefined; connectionForm = true; composing = false; settings = false; globalError = ''; closeNavigation(); }

  function forget(profile: ConnectionProfile) {
    if (reconnectProfile?.id === profile.id) cancelBootstrap();
    const live = connections.find((connection) => connection.profile.id === profile.id);
    if (live) disconnect(live);
    profiles = profiles.filter((item) => item.id !== profile.id);
    if (reconnectProfile?.id === profile.id) { reconnectProfile = undefined; connectionForm = false; }
    try { saveProfiles(profiles); } catch { globalError = 'This browser could not remove the saved connection. It may reappear after reloading; clear this site’s saved data to forget it permanently.'; }
  }

  async function selectWorkflow(connection: Connection, id: string, cursor?: string) {
    connection.busy = true;
    try {
      const suffix = cursor ? `?cursor=${encodeURIComponent(cursor)}` : '';
      const selected = await connection.client.request<WorkflowReadDto>(`/v1/workflows/${encodeURIComponent(id)}${suffix}`);
      if (connection.stream.signal.aborted) return;
      connection.selected = selected;
      connection.taskCursor = cursor;
    }
    catch (failure) { fail(connection, failure); }
    finally { connection.busy = false; }
  }

  function search(connection: Connection) {
    connection.pageCursor = undefined;
    connection.appliedSearch = connection.search;
    void refresh(connection);
  }

  async function created(connection: Connection, receipt: WorkflowReceipt) {
    if (connection.stream.signal.aborted) return;
    if (activeId === connection.profile.id) composing = false;
    // A confirmed receipt identifies the work even while discovery is slow or being refreshed.
    await selectWorkflow(connection, receipt.workflow_id);
    await refresh(connection);
  }

  onDestroy(() => { cancelBootstrap(); for (const connection of connections) { connection.stream.abort(); connection.client.disconnect(); } });
</script>

<svelte:head><meta name="description" content="Submit work to your Milkdrift daemon, follow its progress, and return to retained results." /></svelte:head>
<a class="skip-link" href="#main">Skip to workspace</a>
<div class="workspace">
  <header class="app-header">
    <img class="mobile-logo" src="/milkdrift-logo.svg" alt="" width="34" height="32" />
    <button class="navigation-toggle" bind:this={navigationButton} aria-label="Daemon navigation" aria-expanded={navigationOpen} aria-controls="daemon-navigation" onclick={() => navigationOpen = !navigationOpen}><span aria-hidden="true">☰</span></button>
    <div class="workspace-context"><span class="eyebrow">{active ? active.session.daemon_name : 'Milkdrift workspace'}</span><span class="context-title">{connectionForm ? 'Connection' : settings && active ? 'Daemon settings' : composing && active ? 'New task' : active?.selected?.title ?? 'Workflows'}</span></div>
    <div class="header-status"><span class="status-dot" class:connected={!!active} aria-hidden="true"></span>{active ? 'Connected' : 'Not connected'}</div>
  </header>
  <aside id="daemon-navigation" class="sidebar" class:navigation-open={navigationOpen} aria-label="Daemon connections">
    <div class="brand-lockup"><img class="brand-logo" src="/milkdrift-logo.svg" alt="" width="46" height="44" /><span class="brand">Milkdrift</span></div>
    <div class="sidebar-heading"><p class="eyebrow">Daemons</p><button class="quiet" onclick={addConnection} aria-label="Add daemon connection">+</button></div>
    <ul class="connection-list">{#each profiles as profile (profile.id)}{@const connected = connections.some((item) => item.profile.id === profile.id)}<li><button class:active={activeId === profile.id} onclick={() => choose(profile)} aria-pressed={activeId === profile.id}><span class="status-dot" class:connected aria-hidden="true"></span><span>{profile.cachedName}<small>{profile.endpoint}</small><small>{connected ? 'Connected' : 'Cached name · Authenticate to reconnect'}</small></span></button><button class="quiet forget-connection" onclick={() => forget(profile)} aria-label={`Forget ${profile.cachedName} connection`}>Forget</button></li>{/each}</ul>
    {#if !profiles.length}<p class="sidebar-footer">Add your first daemon to start a task.</p>{/if}
    {#if active}
      <nav class="workspace-navigation" aria-label="Workspace">
        <button class:active={!settings} aria-current={!settings ? 'page' : undefined} onclick={showWorkflows}><span aria-hidden="true">▦</span> Workflows</button>
        <button class:active={settings} aria-current={settings ? 'page' : undefined} onclick={showSettings}><span aria-hidden="true">⚙</span> Daemon settings</button>
      </nav>
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
        {#if settings}
          <DaemonSettings client={active.client} session={active.session} bind:draft={active.rename} onrenamed={renamed.bind(null, active)} onrefresh={refreshSession.bind(null, active)} onerror={fail.bind(null, active)} onback={() => settings = false} />
        {:else if composing}
          <TaskComposer client={active.client} session={active.session} oncreated={created.bind(null, active)} oncancel={() => composing = false} onerror={fail.bind(null, active)} />
        {:else if active.selected}
          <WorkflowView workflow={active.selected} client={active.client} capabilities={active.session.capabilities} onrefresh={refresh.bind(null, active)} onpage={selectWorkflow.bind(null, active, active.selected.id)} onback={() => active!.selected = null} onerror={fail.bind(null, active)} />
        {:else}
          <div class="page-heading"><div><p class="eyebrow">Workspace</p><h1>Your workflows</h1><p>Work in progress and retained results on {active.session.daemon_name}.</p></div><button class="primary" onclick={() => composing = true}>+ New task</button></div>
          <form class="toolbar" onsubmit={(event) => { event.preventDefault(); search(active!); }}><label><span class="visually-hidden">Search workflows and retained results</span><input type="search" bind:value={active.search} placeholder="Search workflows and retained results" /></label><button>Search</button><button type="button" onclick={() => refresh(active!)}>Refresh</button></form>
          {#if active.workflows.length}
            <ul class="workflow-list">{#each active.workflows as workflow (workflow.id)}<li><button class="workflow-card" onclick={() => selectWorkflow(active!, workflow.id)} disabled={active.busy}><span><h3>{workflow.title}</h3><p>{workflow.description}</p><small>{workflow.task_count} {workflow.task_count === 1 ? 'task' : 'tasks'} · Updated {timeLabel(workflow.updated_at)}</small></span><span><span class="badge {workflow.state}">{workflow.state}</span></span></button></li>{/each}</ul>
          {:else}
            <div class="panel empty"><div class="empty-symbol" aria-hidden="true">↗</div><h2>{active.appliedSearch ? 'No matching workflows' : 'A clear space for your next task'}</h2><p>{active.appliedSearch ? 'The daemon returned no accessible workflows for this search. Try a different title or result phrase.' : 'Give your daemon instructions and context. Your tasks and their results will appear here.'}</p>{#if !active.appliedSearch}<button class="primary" onclick={() => composing = true}>Start your first task</button>{/if}</div>
          {/if}
          {#if active.truncated}<p class="notice warning">The daemon’s bounded search window was exhausted. Refine the search to inspect more specific work.</p>{/if}
          {#if active.pageCursor || active.nextCursor}<div class="form-actions">{#if active.pageCursor}<button onclick={() => { active!.pageCursor = undefined; void refresh(active!); }}>First page</button>{/if}{#if active.nextCursor}<button onclick={() => refresh(active!, true)}>Next page</button>{/if}</div>{/if}
        {/if}
      {/key}
    {:else}
      <div class="page-heading"><div><p class="eyebrow">Your workspace</p><h1>Work continues here.</h1></div></div>
      <div class="panel empty"><div class="empty-symbol" aria-hidden="true">↗</div><h2>Connect to your work</h2><p>Choose a saved daemon or add a connection. Sign in with its scoped credential to discover your tasks and retained results.</p><button class="primary" onclick={addConnection}>Connect a daemon</button></div>
    {/if}
  </main>
</div>
