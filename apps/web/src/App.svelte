<script lang="ts">
  import { onDestroy } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import ConnectionForm from './components/ConnectionForm.svelte';
  import TaskComposer from './components/TaskComposer.svelte';
  import WorkflowView from './components/WorkflowView.svelte';
  import { endpointUrl, loadProfiles, saveProfiles } from './lib/connections';
  import type { ConnectionProfile } from './lib/connections';
  import type { TaskSessionDto, WorkflowPageDto, WorkflowReadDto, WorkflowReceipt } from './lib/protocol.generated';
  import { DaemonClient, DaemonError } from './lib/transport';
  import { timeLabel } from './lib/presentation';

  interface Connection {
    profile: ConnectionProfile;
    client: DaemonClient;
    session: TaskSessionDto;
    workflows: WorkflowReadDto[];
    nextCursor: string | null;
    truncated: boolean;
    search: string;
    selected: WorkflowReadDto | null;
    error: string;
    observation: string;
    stream: AbortController;
    cursor: string | undefined;
    refreshGeneration: number;
  }

  let profiles = $state(loadProfiles());
  let connections = $state<Connection[]>([]);
  let activeId = $state('');
  let connectionForm = $state(false);
  let reconnectProfile = $state<ConnectionProfile | undefined>();
  let composing = $state(false);
  let busy = $state(false);
  let globalError = $state('');
  let active = $derived(connections.find((connection) => connection.profile.id === activeId));

  function fail(connection: Connection, failure: unknown) {
    if (failure instanceof Error && failure.name === 'AbortError') return;
    const message = failure instanceof Error ? failure.message : 'The daemon request failed.';
    if (failure instanceof DaemonError && failure.authorizationLost) {
      disconnect(connection);
      globalError = `${connection.profile.label}: ${message} Authenticate again to read permitted work.`;
    } else connection.error = message;
  }

  function disconnect(connection: Connection) {
    connection.stream.abort();
    connection.client.disconnect();
    connection.workflows = [];
    connection.selected = null;
    connection.search = '';
    connection.nextCursor = null;
    connection.cursor = undefined;
    connections = connections.filter((item) => item.profile.id !== connection.profile.id);
    if (activeId === connection.profile.id) { activeId = ''; composing = false; }
  }

  async function refresh(connection: Connection, append = false) {
    const generation = ++connection.refreshGeneration;
    const query = new SvelteURLSearchParams({ limit: '50', search: connection.search });
    if (append && connection.nextCursor) query.set('cursor', connection.nextCursor);
    try {
      const page = await connection.client.request<WorkflowPageDto>(`/v1/workflows?${query}`);
      if (generation !== connection.refreshGeneration || connection.stream.signal.aborted) return;
      connection.workflows = append ? [...connection.workflows, ...page.items] : page.items;
      connection.nextCursor = page.next_cursor;
      connection.truncated = page.truncated;
      connection.error = '';
      if (connection.selected) {
        const selectedId = connection.selected.id;
        const snapshot = await connection.client.request<WorkflowReadDto>(`/v1/workflows/${encodeURIComponent(selectedId)}`);
        if (generation === connection.refreshGeneration && connection.selected?.id === selectedId) connection.selected = snapshot;
      }
    } catch (failure) {
      if (failure instanceof DaemonError && (failure.status === 410 || failure.code === 'expired_cursor')) {
        connection.nextCursor = null;
        connection.cursor = undefined;
        if (append) await refresh(connection);
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
            resync = true;
            await refresh(connection);
          } else if (event.event === 'workflow_changed') {
            if (event.id) connection.cursor = event.id;
            await refresh(connection);
          } else if (event.event === 'forbidden') {
            fail(connection, new DaemonError('unauthorized', 'Observation authority changed.', 403));
          }
        }, signal, connection.cursor);
      } catch (failure) {
        if (signal.aborted) return;
        if (failure instanceof DaemonError && (failure.status === 410 || failure.code === 'expired_cursor')) {
          connection.cursor = undefined;
          resync = true;
          await refresh(connection);
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

  async function connect(details: { label: string; endpoint: string; daemonId: string; token: string }) {
    const endpoint = endpointUrl(details.endpoint);
    const client = new DaemonClient(endpoint, details.token);
    try {
      const session = await client.request<TaskSessionDto>('/v1/session');
      if (session.daemon_id !== details.daemonId) throw new Error('Installation identity does not match this daemon. Check the endpoint and the identity in your daemon configuration.');
      client.bindIdentity(session.daemon_id);
      const previous = reconnectProfile;
      if (previous) {
        const live = connections.find((item) => item.profile.id === previous.id);
        if (live) disconnect(live);
      }
      const profile = { id: previous?.id ?? crypto.randomUUID(), label: details.label, endpoint, daemonId: session.daemon_id };
      profiles = [...profiles.filter((item) => item.id !== profile.id), profile];
      try { saveProfiles(profiles); } catch { globalError = 'This browser could not save the connection name. The authenticated connection still works for this session.'; }
      const connection: Connection = {
        profile, client, session, workflows: [], nextCursor: null, truncated: false, search: '', selected: null,
        error: '', observation: 'Connecting live updates', stream: new AbortController(), cursor: undefined, refreshGeneration: 0,
      };
      connections = [...connections, connection];
      activeId = profile.id;
      connectionForm = false;
      composing = false;
      // Read and update through the reactive connection proxy after insertion.
      const installed = connections.find((item) => item.profile.id === profile.id)!;
      await refresh(installed);
      void observe(installed);
    } catch (failure) { client.disconnect(); throw failure; }
  }

  function choose(profile: ConnectionProfile) {
    globalError = '';
    composing = false;
    const live = connections.find((item) => item.profile.id === profile.id);
    if (live) { activeId = profile.id; connectionForm = false; }
    else { reconnectProfile = profile; connectionForm = true; }
  }

  function addConnection() { reconnectProfile = undefined; connectionForm = true; composing = false; globalError = ''; }

  async function selectWorkflow(connection: Connection, id: string) {
    busy = true;
    try { connection.selected = await connection.client.request<WorkflowReadDto>(`/v1/workflows/${encodeURIComponent(id)}`); }
    catch (failure) { fail(connection, failure); }
    finally { busy = false; }
  }

  async function created(connection: Connection, receipt: WorkflowReceipt) {
    composing = false;
    await refresh(connection);
    await selectWorkflow(connection, receipt.workflow_id);
  }

  onDestroy(() => { for (const connection of connections) { connection.stream.abort(); connection.client.disconnect(); } });
</script>

<svelte:head><meta name="description" content="Submit work to your Milkdrift daemon, follow its progress, and return to retained results." /></svelte:head>
<a class="skip-link" href="#main">Skip to workspace</a>
<header class="app-header"><span class="brand-mark" aria-hidden="true">m</span><span class="brand">milkdrift</span><span class="header-note">Your work keeps moving.</span></header>
<div class="workspace">
  <aside class="sidebar" aria-label="Daemon connections">
    <div class="sidebar-heading"><p class="eyebrow">Daemons</p><button class="quiet" onclick={addConnection} aria-label="Add daemon connection">+</button></div>
    <ul class="connection-list">{#each profiles as profile (profile.id)}{@const connected = connections.some((item) => item.profile.id === profile.id)}<li><button class:active={activeId === profile.id} onclick={() => choose(profile)} aria-pressed={activeId === profile.id}><span class="status-dot" class:connected aria-hidden="true"></span><span>{profile.label}<small>{connected ? 'Connected' : 'Authenticate to reconnect'}</small></span></button></li>{/each}</ul>
    {#if !profiles.length}<p class="sidebar-footer">Add your first daemon to start a task.</p>{/if}
    {#if active}<div class="connection-detail"><div><p>{active.session.actor_id} · {active.session.daemon_id}</p><p aria-live="polite">{active.observation}</p></div><button class="quiet" onclick={() => disconnect(active!)}>Disconnect</button></div>{/if}
    <p class="sidebar-footer">Each connection has its own credentials, work and settings. Closing this client leaves your daemon running.</p>
  </aside>
  <main id="main">
    {#if globalError}<p class="notice error" role="alert">{globalError}</p>{/if}
    {#if connectionForm}
      <ConnectionForm profile={reconnectProfile} onconnect={connect} oncancel={() => connectionForm = false} />
    {:else if active}
      {#if active.error}<p class="notice error" role="alert">{active.error}</p>{/if}
      {#key active.profile.id}
        {#if composing}
          <TaskComposer client={active.client} session={active.session} oncreated={created.bind(null, active)} oncancel={() => composing = false} onerror={fail.bind(null, active)} />
        {:else if active.selected}
          <WorkflowView workflow={active.selected} client={active.client} onrefresh={refresh.bind(null, active)} onback={() => active!.selected = null} onerror={fail.bind(null, active)} />
        {:else}
          <div class="page-heading"><div><p class="eyebrow">{active.profile.label}</p><h1>Your workflows</h1><p>Start something useful. Come back when it’s ready.</p></div><button class="primary" onclick={() => composing = true}>+ New task</button></div>
          <form class="toolbar" onsubmit={(event) => { event.preventDefault(); void refresh(active!); }}><label><span class="visually-hidden">Search workflows and retained results</span><input type="search" bind:value={active.search} placeholder="Search workflows and retained results" /></label><button>Search</button><button type="button" onclick={() => refresh(active!)}>Refresh</button></form>
          {#if active.workflows.length}
            <ul class="workflow-list">{#each active.workflows as workflow (workflow.id)}<li><button class="workflow-card" onclick={() => selectWorkflow(active!, workflow.id)} disabled={busy}><span><h3>{workflow.title}</h3><p>{workflow.description}</p><small>{workflow.tasks.length} {workflow.tasks.length === 1 ? 'task' : 'tasks'} · Updated {timeLabel(workflow.updated_at)}</small></span><span><span class="badge {workflow.state}">{workflow.state}</span></span></button></li>{/each}</ul>
          {:else}
            <div class="panel empty"><div class="empty-symbol" aria-hidden="true">↗</div><h2>{active.search ? 'No matching workflows' : 'A clear space for your next task'}</h2><p>{active.search ? 'The daemon returned no accessible workflows for this search. Try a different title or result phrase.' : 'Give your daemon instructions and context. Your tasks and their results will appear here.'}</p>{#if !active.search}<button class="primary" onclick={() => composing = true}>Start your first task</button>{/if}</div>
          {/if}
          {#if active.truncated}<p class="notice warning">The daemon’s bounded search window was exhausted. Refine the search to inspect more specific work.</p>{/if}
          {#if active.nextCursor}<div class="form-actions"><button onclick={() => refresh(active!, true)}>Load more workflows</button></div>{/if}
        {/if}
      {/key}
    {:else}
      <div class="page-heading"><div><p class="eyebrow">A workspace that stays with you</p><h1>Start here. Keep going anywhere.</h1></div></div>
      <div class="panel empty"><div class="empty-symbol" aria-hidden="true">↗</div><h2>Connect to your work</h2><p>Choose a saved daemon or add a connection. Sign in with its scoped credential to discover your tasks and retained results.</p><button class="primary" onclick={addConnection}>Connect a daemon</button></div>
    {/if}
  </main>
</div>
