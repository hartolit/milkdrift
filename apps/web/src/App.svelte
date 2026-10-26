<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import ConnectionForm from './components/ConnectionForm.svelte';
  import DaemonTree from './components/DaemonTree.svelte';
  import DaemonSettings, { type RenameDraft } from './components/DaemonSettings.svelte';
  import CanvasWorkspace from './components/CanvasWorkspace.svelte';
  import ModelsAgents from './components/ModelsAgents.svelte';
  import ProjectSettings, { newProjectDraft, type ProjectDraft } from './components/ProjectSettings.svelte';
  import { endpointUrl, loadProfiles, maxConnections, saveProfiles } from './lib/connections';
  import type { ConnectionProfile } from './lib/connections';
  import type { CreateWorkflowRequest, ProjectCommandDto, ProjectDto, ProjectPageDto, ProjectReceiptDto, RenameDaemonReceipt, TaskSessionDto, WorkflowPageDto, WorkflowReadDto, WorkflowReceipt, WorkflowSummaryDto } from './lib/protocol.generated';
  import { createCanvas, hasChanges, type CanvasState as TaskDraft, type CanvasTarget as TaskDraftTarget } from './lib/canvas';
  import { DaemonClient, DaemonError } from './lib/transport';
  import { timeLabel } from './lib/presentation';

  interface Connection {
    profile: ConnectionProfile;
    client: DaemonClient;
    session: TaskSessionDto;
    rename: RenameDraft;
    projects: ProjectDto[];
    projectCursor: string | null;
    projectId: string;
    earlierOnly: boolean;
    projectDraft: ProjectDraft;
    projectDrafts: ProjectDraft[];
    projectGeneration: number;
    workGeneration: number;
    unresolvedProjects: ProjectReceiptDto[];
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
    newWorkflow: CreateWorkflowRequest | null;
    view: 'work' | 'settings' | 'models' | 'project';
    loading: boolean;
    discoveryKey: string | undefined;
    taskCursor: string | undefined;
    error: string;
    observation: string;
    stream: AbortController;
    metadataTimer: number | null;
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
  let navigationButton: HTMLButtonElement | undefined;
  let bootstrap: DaemonClient | undefined;
  let bootstrapGeneration = 0;
  let active = $derived(connections.find((connection) => connection.profile.id === activeId));
  let visibleDraft = $derived(active?.drafts.find((draft) => draft.id === active.visibleDraftId));
  let activeProject = $derived(active?.projects.find((project) => project.id === active.projectId) ?? null);
  const maxDrafts = 8;
  let workflowView: { canNavigate: () => boolean; addTask: () => Promise<void> } | undefined = $state();

  $effect(() => {
    const states = connections.flatMap((connection) => {
      const dirtyCount = connection.drafts.filter((draft) => draft.authoringSave.status !== 'saved' && draft.authoringSave.status !== 'loading' || draft.pending).length;
      return connection.drafts.map((draft) => ({ draft, blocked: dirtyCount >= maxDrafts && !hasChanges(draft) }));
    });
    untrack(() => { for (const { draft, blocked } of states) draft.capacityBlocked = blocked; });
  });

  function mayLeaveWork() {
    return active?.view !== 'work' || connectionForm || !active?.selected || !workflowView || workflowView.canNavigate();
  }

  interface Location { connection: string; daemon: string; project: string; earlierOnly: boolean; workflow: string; view: Connection['view']; draft: string }
  function rememberLocation(replace = false) {
    const location: Location | null = active ? { connection: active.profile.id, daemon: active.session.daemon_id, project: active.projectId, earlierOnly: active.earlierOnly, workflow: active.selectedId, view: active.view, draft: active.visibleDraftId } : null;
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
    if (location.view === 'project') {
      const draft = ensureProjectDraft(connection, location.project || null);
      if (!draft) { rememberLocation(true); return; }
      connection.projectDraft = draft;
    }
    activeId = connection.profile.id;
    connection.view = location.view === 'settings' || location.view === 'models' || location.view === 'project' ? location.view : 'work';
    connection.projectId = location.project || '';
    connection.earlierOnly = location.earlierOnly === true;
    connection.visibleDraftId = connection.drafts.some((draft) => draft.id === location.draft) ? location.draft : '';
    void selectWorkflow(connection, location.workflow || '', undefined, false, false);
    void refreshProjects(connection);
    if (!location.workflow) void refresh(connection);
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

  function showModels() {
    closeConnectionForm();
    if (active) { active.view = 'models'; rememberLocation(); }
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
    if (connection.metadataTimer !== null) window.clearInterval(connection.metadataTimer);
    connection.client.disconnect();
    disposeWork(connection);
    connection.workflows = [];
    connection.discoveryKey = undefined;
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
    connection.workGeneration++;
    connection.projectGeneration++;
    connection.selectionGeneration++;
    for (const draft of connection.drafts) { draft.disposed = true; draft.checkpoint?.close(); draft.positionCheckpoint?.close(); }
    connection.drafts = [];
    connection.visibleDraftId = '';
    connection.newWorkflow = null;
    connection.recent = [];
    connection.projects = [];
    connection.projectId = '';
    connection.earlierOnly = false;
    connection.projectDraft = newProjectDraft();
    connection.projectDrafts = [];
    connection.projectCursor = null;
    if (connection.view === 'project') connection.view = 'work';
    connection.unresolvedProjects = [];
    connection.workflows = [];
    connection.discoveryKey = undefined;
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
        connection.rename = { name: session.daemon_name, dirty: false, pending: null };
      }
      const name = session.daemon_name_generation >= connection.session.daemon_name_generation
        ? session : connection.session;
      connection.session = { ...session, daemon_name: name.daemon_name, daemon_name_generation: name.daemon_name_generation };
      cacheName(connection, connection.session.daemon_name);
    } catch (failure) { fail(connection, failure); }
  }

  async function refreshObserved(connection: Connection) {
    // Metadata remains observable even when workflow discovery is slow or unavailable.
    await refreshSession(connection);
    if (!connection.stream.signal.aborted) await Promise.all([refreshProjects(connection), refresh(connection)]);
  }

  async function refreshProjects(connection: Connection, nextPage = false) {
    const generation = ++connection.projectGeneration;
    const cursor = nextPage ? connection.projectCursor : null;
    try {
      const page = await connection.client.request<ProjectPageDto>(`/v1/projects?limit=30${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''}`);
      if (connection.stream.signal.aborted || generation !== connection.projectGeneration) return;
      connection.projects = nextPage ? [...connection.projects, ...page.items.filter((item) => !connection.projects.some((known) => known.id === item.id))].slice(-120) : page.items;
      connection.projectCursor = page.next_cursor;
      if (connection.projectId && !connection.projects.some((item) => item.id === connection.projectId)) {
        const selected = await connection.client.request<ProjectDto>(`/v1/projects/${encodeURIComponent(connection.projectId)}`);
        if (!connection.stream.signal.aborted && generation === connection.projectGeneration) connection.projects = [...connection.projects.slice(0, 119), selected];
      }
      const operations = await connection.client.request<ProjectReceiptDto[]>('/v1/project-operations');
      if (!connection.stream.signal.aborted && generation === connection.projectGeneration) connection.unresolvedProjects = operations;
    } catch (failure) {
      if (generation !== connection.projectGeneration) return;
      if (failure instanceof DaemonError && (failure.status === 403 || failure.status === 404)) {
        connection.refreshGeneration++;
        disposeWork(connection);
        connection.loading = false;
      }
      fail(connection, failure);
    }
  }

  function ensureProjectDraft(connection: Connection, projectId: string | null) {
    let draft = connection.projectDrafts.find((item) => item.projectId === projectId);
    if (!draft) {
      if (connection.projectDrafts.length >= maxDrafts) {
        const clean = connection.projectDrafts.find((item) => !item.dirty && !item.pending && !item.recovery);
        if (!clean) {
          connection.error = `This connection holds ${maxDrafts} project drafts. Open and discard or recover one before starting another; your existing drafts are intact.`;
          return;
        }
        connection.projectDrafts = connection.projectDrafts.filter((item) => item !== clean);
      }
      connection.projectDrafts = [...connection.projectDrafts, newProjectDraft(projectId)];
      draft = connection.projectDrafts.find((item) => item.projectId === projectId)!;
    }
    return draft;
  }

  function showProject(connection: Connection, project: ProjectDto | null) {
    if (!mayLeaveWork()) return;
    const draft = ensureProjectDraft(connection, project?.id ?? null);
    if (!draft) return;
    if (connection.selected?.project_id !== project?.id) clearSelectedWorkflow(connection);
    connection.projectId = project?.id ?? '';
    connection.projectDraft = draft;
    connection.view = 'project';
    closeConnectionForm(); closeNavigation(); rememberLocation();
  }

  function recoverProject(connection: Connection, operation: ProjectReceiptDto) {
    showProject(connection, null);
    if (connection.projectDraft.projectId !== null || connection.view !== 'project') return;
    if (connection.projectDraft.pending || connection.projectDraft.dirty) {
      connection.error = 'Recover or discard the current new-project draft before opening another folder recovery.';
      return;
    }
    connection.projectDraft.recovery = { operationId: operation.operation_id!, folder: operation.operation_folder };
  }

  async function associateWorkflow(connection: Connection, project: ProjectDto) {
    if (!connection.selected) return;
    const draft = ensureProjectDraft(connection, project.id);
    if (!draft || draft.pending) return;
    const generation = connection.workGeneration;
    const workflowId = connection.selected.id;
    const request = { request_id: crypto.randomUUID(), command: { kind: 'associate' as const, project_id: project.id, expected_generation: project.generation, workflow_id: connection.selected.id } };
    draft.pending = request;
    connection.projectDraft = draft;
    const current = () => !connection.stream.signal.aborted && generation === connection.workGeneration && connection.projectDrafts.includes(draft);
    try {
      const receipt = await connection.client.request<ProjectReceiptDto>('/v1/project-commands', request);
      if (!current()) return;
      if (receipt.request_id !== request.request_id) throw new Error('The reply belongs to a different project request.');
      draft.pending = null;
      await projectAccepted(connection, draft, receipt, request.command, connection.selectedId === workflowId && connection.view === 'work');
    } catch (failure) {
      if (!current()) return;
      if (failure instanceof DaemonError && failure.status < 500) draft.pending = null;
      fail(connection, failure);
      if (draft.pending && connection.selectedId === workflowId && connection.view === 'work') showProject(connection, project);
    }
  }

  async function chooseProject(connection: Connection, id: string, earlierOnly = false) {
    if (!mayLeaveWork()) return;
    connection.projectId = id;
    connection.earlierOnly = earlierOnly;
    connection.pageCursor = undefined; connection.nextCursor = null;
    connection.search = ''; connection.appliedSearch = '';
    connection.recent = [];
    await selectWorkflow(connection, '');
    await refresh(connection);
  }

  async function projectAccepted(connection: Connection, draft: ProjectDraft, receipt: ProjectReceiptDto, command: ProjectCommandDto, navigate = true) {
    const generation = connection.workGeneration;
    const projectId = connection.projectId;
    const selectedId = connection.selectedId;
    const view = connection.view;
    const current = () => !connection.stream.signal.aborted && generation === connection.workGeneration && connection.projectDrafts.includes(draft);
    if (!current()) return;
    if (command.kind === 'remove') {
      // A removal receipt retains the original project. It proves the command, not a live registration.
      connection.projects = connection.projects.filter((item) => item.id !== command.project_id);
      connection.projectDrafts = connection.projectDrafts.filter((item) => item.projectId !== command.project_id);
      if (connection.projectDraft.projectId === command.project_id) connection.projectDraft = newProjectDraft();
      if (connection.projectId === command.project_id) {
        connection.projectId = '';
        connection.earlierOnly = false;
        connection.pageCursor = undefined;
        connection.nextCursor = null;
        if (connection.view === 'project') connection.view = 'work';
      }
      for (const drawing of connection.drafts) {
        if (drawing.target.kind === 'new' && drawing.target.projectId === command.project_id) {
          drawing.target.projectId = undefined;
          drawing.project = null;
          drawing.notice = 'This project registration was removed. Your unsent drawing is retained for inspection or discard; start new work in a registered project.';
        }
      }
      await refreshProjects(connection);
      if (connection.stream.signal.aborted || generation !== connection.workGeneration) return;
      await refresh(connection);
      if (!connection.stream.signal.aborted && generation === connection.workGeneration && active === connection) rememberLocation(true);
      return;
    }
    await refreshProjects(connection);
    if (!current()) return;
    if (receipt.workflow_id) {
      for (const draft of connection.drafts.filter((item) => item.target.kind === 'existing' && item.target.workflowId === receipt.workflow_id)) {
        draft.target.projectId = receipt.project?.id ?? null;
        draft.project = receipt.project;
      }
    }
    if (!navigate || active !== connection || connection.projectDraft !== draft || connection.projectId !== projectId || connection.selectedId !== selectedId || connection.view !== view) return;
    if (receipt.project) {
      if (!receipt.workflow_id && connection.selected?.project_id !== receipt.project.id) clearSelectedWorkflow(connection);
      connection.projectId = receipt.project.id;
    }
    if (receipt.workflow_id) {
      await selectWorkflow(connection, receipt.workflow_id);
    } else if (receipt.project && draft.mode !== 'settings') {
      // The new-folder draft is resolved, so replace its slot with the accepted project's editor.
      connection.projectDrafts = connection.projectDrafts.filter((item) => item !== draft);
      const acceptedDraft = ensureProjectDraft(connection, receipt.project.id);
      if (acceptedDraft) connection.projectDraft = acceptedDraft;
    } else if (!receipt.project && receipt.status === 'accepted') {
      connection.projectId = '';
      connection.view = 'work';
    }
    await refresh(connection);
    rememberLocation(true);
  }

  async function refresh(connection: Connection, nextPage = false) {
    const generation = ++connection.refreshGeneration;
    const pageCursor = nextPage ? connection.nextCursor ?? undefined : connection.pageCursor;
    const query = new SvelteURLSearchParams({ limit: '50', search: connection.appliedSearch });
    if (connection.projectId) query.set('project_id', connection.projectId);
    else if (connection.earlierOnly) query.set('earlier', 'true');
    if (pageCursor) query.set('cursor', pageCursor);
    const discoveryKey = query.toString();
    // Stream replay refreshes the same snapshot repeatedly. Keep an already loaded empty result
    // visible; only a different search/page or the initial read needs the loading placeholder.
    connection.loading = connection.discoveryKey !== discoveryKey;
    try {
      const page = await connection.client.request<WorkflowPageDto>(`/v1/workflows?${query}`);
      if (generation !== connection.refreshGeneration || connection.stream.signal.aborted) return;
      connection.workflows = page.items;
      connection.pageCursor = pageCursor;
      connection.nextCursor = page.next_cursor;
      connection.truncated = page.truncated;
      connection.discoveryKey = discoveryKey;
      connection.loading = false;
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
      const page = await connection.client.request<WorkflowPageDto>(`/v1/workflows?limit=8&search=${connection.projectId ? `&project_id=${encodeURIComponent(connection.projectId)}` : connection.earlierOnly ? '&earlier=true' : ''}`);
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
            await refresh(connection);
          } else if (event.event === 'metadata_changed') {
            await refreshSession(connection);
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
        selectedId: '', selectionGeneration: 0, sessionGeneration: 0, projects: [], projectCursor: null, projectId: '', earlierOnly: false, projectDraft: newProjectDraft(), projectDrafts: [], projectGeneration: 0, workGeneration: 0, unresolvedProjects: [], recent: [], drafts: [], visibleDraftId: '', view: 'work', loading: true, discoveryKey: undefined,
        newWorkflow: null, error: '', observation: 'Connecting live updates', stream: new AbortController(), metadataTimer: null, cursor: undefined, refreshGeneration: 0, busy: false,
      };
      connections = [...connections, connection];
      activeId = profile.id;
      connectionForm = false;
      bootstrap = undefined;
      // Read and update through the reactive connection proxy after insertion.
      const installed = connections.find((item) => item.profile.id === profile.id)!;
      const restored = window.history.state?.milkdrift as Location | undefined;
      if (restored?.connection === profile.id && restored.daemon === session.daemon_id) {
        installed.view = restored.view;
        installed.projectId = restored.project || '';
        if (restored.view === 'project') installed.projectDraft = ensureProjectDraft(installed, restored.project || null)!;
        void selectWorkflow(installed, restored.workflow, undefined, false);
      }
      rememberLocation();
      installed.metadataTimer = window.setInterval(() => { if (!installed.stream.signal.aborted) void Promise.all([refreshSession(installed), refreshProjects(installed)]); }, 30000);
      void observe(installed);
      await Promise.all([refreshProjects(installed), refresh(installed)]);
    } catch (failure) { client.disconnect(); throw failure; }
  }

  function withConnection(id: string, action: (connection: Connection) => void) {
    const connection = connections.find((item) => item.profile.id === id);
    if (!connection || !mayLeaveWork()) return;
    activeId = id; closeConnectionForm(); action(connection);
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
      if (connection.view === 'work') connection.projectId = selected.project_id ?? '';
      if (connection.view === 'work' && !connection.visibleDraftId) {
        const canvas = ensureCanvas(connection, { kind: 'existing', workflowId: selected.id, title: selected.title, generation: selected.generation, projectId: selected.project_id });
        if (canvas) connection.visibleDraftId = canvas.id;
      }
      const summary: WorkflowSummaryDto = { id: selected.id, project_id: selected.project_id, title: selected.title, description: selected.description, generation: selected.generation, state: selected.state, created_at: selected.created_at, updated_at: selected.updated_at, task_count: selected.task_count };
      connection.recent = visiting
        ? [summary, ...connection.recent.filter((item) => item.id !== id)].slice(0, 8)
        : connection.recent.map((item) => item.id === id ? summary : item);
    }
    catch (failure) { if (generation === connection.selectionGeneration) fail(connection, failure); }
    finally { if (generation === connection.selectionGeneration) connection.busy = false; }
  }

  function clearSelectedWorkflow(connection: Connection) {
    connection.selectionGeneration++;
    connection.selectedId = ''; connection.selected = null; connection.visibleDraftId = '';
    connection.taskCursor = undefined; connection.busy = false;
    connection.pageCursor = undefined; connection.nextCursor = null;
    connection.search = ''; connection.appliedSearch = ''; connection.recent = [];
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

  function ensureCanvas(connection: Connection, target: TaskDraftTarget) {
    let draft = connection.drafts.find((item) => item.target.kind === target.kind && (target.kind === 'new' ? item.target.projectId === target.projectId : (item.target.kind === 'existing' && item.target.workflowId === target.workflowId)));
    if (!draft) {
      if (connection.drafts.length >= maxDrafts + 1) {
        const clean = connection.drafts.find((item) => item.authoringSave.status === 'saved' && !item.pending && item.id !== connection.visibleDraftId);
        if (clean) { clean.disposed = true; clean.checkpoint?.close(); clean.positionCheckpoint?.close(); connection.drafts = connection.drafts.filter((item) => item !== clean); }
        else { connection.error = `This connection holds ${maxDrafts} drafts. Open and discard one before starting another; your existing drafts are intact.`; return; }
      }
      draft = createCanvas(connection.session, target, connection.projects.find((project) => project.id === target.projectId) ?? null);
      connection.drafts = [...connection.drafts, draft];
    }
    return connection.drafts.find((item) => item.id === draft.id)!;
  }

  async function openComposer(connection: Connection, target: TaskDraftTarget) {
    if (target.kind !== 'new') { await selectWorkflow(connection, target.workflowId); return; }
    const projectId = target.projectId ?? connection.projectId;
    if (!projectId) { showProject(connection, null); return; }
    if (connection.busy) return;
    connection.busy = true; connection.error = '';
    const request = connection.newWorkflow ?? { request_id: crypto.randomUUID(), project_id: projectId, title: 'Untitled workflow', description: '', tasks: [], inputs: {}, finish_when_settled: false, limits: null };
    connection.newWorkflow = request;
    try {
      let receipt: WorkflowReceipt;
      try { receipt = await connection.client.request<WorkflowReceipt>('/v1/workflows', request); }
      catch (failure) {
        if (failure instanceof DaemonError && failure.status < 500) { connection.newWorkflow = null; throw failure; }
        receipt = await connection.client.request<WorkflowReceipt>(`/v1/workflow-requests/${encodeURIComponent(request.request_id)}`);
      }
      if (connection.stream.signal.aborted) return;
      connection.newWorkflow = null; connection.projectId = request.project_id ?? '';
      await selectWorkflow(connection, receipt.workflow_id); await refresh(connection);
    } catch (failure) { fail(connection, failure); }
    finally { connection.busy = false; }
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
    if (draft.target.kind === 'existing') draft.target.generation = receipt.generation;
    draft.conflict = false; draft.error = ''; draft.notice = 'Plan accepted. Other ideas remain saved.';
    if (connection.selectedId === receipt.workflow_id) await readSelection(connection);
    await refresh(connection);
  }

  onDestroy(() => { cancelBootstrap(); for (const connection of connections) { connection.stream.abort(); if (connection.metadataTimer !== null) window.clearInterval(connection.metadataTimer); connection.client.disconnect(); } });
</script>

<svelte:head><meta name="description" content="Submit work to your Milkdrift daemon, follow its progress, and return to retained results." /></svelte:head>
<a class="skip-link" href="#main">Skip to workspace</a>
<div class="workspace" class:canvas-active={!!active && active.view === 'work' && !!visibleDraft && !connectionForm}>
  <header class="app-header">
    <img class="mobile-logo" src="/milkdrift-logo.svg" alt="" width="34" height="32" />
    <button class="navigation-toggle" bind:this={navigationButton} aria-label="Workspace navigation" aria-expanded={navigationOpen} aria-controls="daemon-navigation" onclick={() => navigationOpen = !navigationOpen}><span aria-hidden="true">☰</span> Navigate</button>
    <div class="workspace-context"><span class="eyebrow">{active ? `${active.session.daemon_name}${activeProject ? ` / ${activeProject.name}` : ''}` : 'Milkdrift workspace'}</span><span class="context-title">{connectionForm ? 'Connection' : active?.view === 'settings' ? 'Daemon settings' : active?.view === 'models' ? 'Models & agents' : active?.view === 'project' ? activeProject ? 'Project settings' : 'Add project' : visibleDraft?.target.kind === 'new' ? 'Unsaved workflow' : active?.selected?.title ?? (active?.selectedId ? 'Loading workflow…' : activeProject ? 'All workflows' : 'Projects')}</span></div>
    <div class="header-status"><span class="status-dot" class:connected={!!active} aria-hidden="true"></span>{active ? 'Connected' : 'Not connected'}</div>
  </header>
  <aside id="daemon-navigation" class="sidebar" class:navigation-open={navigationOpen} aria-label="Workspace navigation">
    <div class="brand-lockup"><img class="brand-logo" src="/milkdrift-logo.svg" alt="" width="46" height="44" /><span class="brand">Milkdrift</span></div>
    <div class="sidebar-heading"><p class="eyebrow">Daemon</p><button class="quiet" onclick={addConnection} aria-label="Add daemon connection">+</button></div>
    <DaemonTree {profiles} {connections} {activeId}
      onchoose={choose}
      onselect={(id, workflow) => withConnection(id, (connection) => { void selectWorkflow(connection, workflow); })}
      onnew={(id, project) => withConnection(id, (connection) => { connection.projectId = project.id; openComposer(connection, { kind: 'new', projectId: project.id }); })}
      onproject={(id, project) => withConnection(id, (connection) => showProject(connection, project))}
      onmoreprojects={(id) => { const connection = connections.find((item) => item.profile.id === id); if (connection) void refreshProjects(connection, true); }}
      onsettings={(id) => withConnection(id, () => showSettings())}
      onmodels={(id) => withConnection(id, () => showModels())}
      onerror={(id, failure) => { const connection = connections.find((item) => item.profile.id === id); if (connection) fail(connection, failure); }} />
    {#if !profiles.length}<p class="sidebar-footer">Add your first daemon to start a task.</p>{/if}
    <details class="saved-connections"><summary>Manage connections</summary>{#each profiles as profile (profile.id)}<button class="quiet" onclick={() => forget(profile)} aria-label={`Forget ${profile.cachedName} connection`}>Forget {profile.cachedName}</button>{/each}</details>
    {#if active}
      <button class="quiet" onclick={showWorkflows}>All workflows</button>
      <div class="connection-detail"><p class="eyebrow">Connected daemon</p><strong>{active.session.daemon_name}</strong><p class="connection-address">{active.profile.endpoint}</p><p aria-live="polite"><span class="status-dot connected" aria-hidden="true"></span> {active.observation}</p><button class="quiet" onclick={() => { disconnect(active!); closeNavigation(); }}>Disconnect</button></div>
    {/if}
    <p class="sidebar-footer">Closing this client leaves your daemon running.</p>
  </aside>
  <main id="main" class:canvas-main={!!active && active.view === 'work' && !!visibleDraft && !connectionForm}>
    {#if globalError}<p class="notice error" role="alert">{globalError}</p>{/if}
    {#if connectionForm}
      {#key reconnectProfile?.id}<ConnectionForm profile={reconnectProfile} onconnect={connect} oncancel={closeConnectionForm} />{/key}
    {:else if active}
      {#if active.error}<p class="notice error" role="alert">{active.error}</p>{/if}{#if active.newWorkflow}<div class="notice warning">Workflow creation is unconfirmed. Its exact request is retained.<button disabled={active.busy} onclick={() => openComposer(active!, { kind: 'new', projectId: active!.newWorkflow!.project_id })}>Recover workflow creation</button></div>{/if}
      {#key `${active.profile.id}:${active.session.actor_id}`}
        {#if active.view === 'settings'}
          <DaemonSettings client={active.client} session={active.session} bind:draft={active.rename} onrenamed={renamed.bind(null, active)} onrefresh={refreshSession.bind(null, active)} onerror={fail.bind(null, active)} onback={() => { active!.view = 'work'; rememberLocation(); }} />
        {:else if active.view === 'models'}
          <ModelsAgents client={active.client} session={active.session} onrefresh={refreshSession.bind(null, active)} onerror={fail.bind(null, active)} onback={() => { active!.view = 'work'; rememberLocation(); }} />
        {:else if active.view === 'project'}
          {#key active.projectDraft}<ProjectSettings client={active.client} session={active.session} project={activeProject} bind:draft={active.projectDraft} onaccepted={projectAccepted.bind(null, active, active.projectDraft)} onrefresh={refreshProjects.bind(null, active)} onerror={fail.bind(null, active)} onback={() => { active!.view = 'work'; rememberLocation(); }} onmodels={showModels} />{/key}
        {:else}
          {#if visibleDraft}
            {@const connection = active}{@const draft = visibleDraft}
            {#if !draft.target.projectId && connection.selected}<div class="notice"><p>No project is available to this connection for later submissions. Accepted tasks retain their recorded tools and paths. Earlier workflows can be associated with an accessible project.</p><label>Associate with project<select value="" disabled={!!connection.projectDraft.pending} onchange={(event) => { const project = connection.projects.find((item) => item.id === event.currentTarget.value); if (project) void associateWorkflow(connection, project); }}><option value="">Choose a project</option>{#each connection.projects as project (project.id)}<option value={project.id}>{project.name}</option>{/each}</select></label></div>{/if}
            {#key draft.id}<CanvasWorkspace bind:this={workflowView} bind:canvas={() => draft, () => {}} currentProject={activeProject} workflow={draft.target.kind === 'existing' && connection.selected?.id === draft.target.workflowId ? connection.selected : null} client={connection.client} session={connection.session} onaccepted={accepted.bind(null, connection, draft)} onreviewed={reviewed.bind(null, connection, draft)} onrefresh={readSelection.bind(null, connection)} onpage={(cursor) => selectWorkflow(connection, connection.selectedId, cursor, false, false)} onerror={composerError.bind(null, connection)} onmodels={showModels} />{/key}
          {:else if active.selectedId}
            <div class="panel" role="status">{active.busy ? 'Loading workflow…' : 'This workflow could not be loaded.'}<button onclick={() => readSelection(active!)}>Retry workflow</button></div>
          {:else}
          {#if !activeProject}<div class="page-heading"><div><p class="eyebrow">{active.session.daemon_name}</p><h1>Your projects</h1><p>Choose a project to start work in its folder on this daemon.</p></div><button class="primary" onclick={() => showProject(active!, null)}>Add project</button></div>
            {#if active.projects.length}<ul class="project-list">{#each active.projects as project (project.id)}<li><button class="workflow-card" aria-label={`Open project ${project.name}`} onclick={() => chooseProject(active!, project.id)}><strong>{project.name}</strong><small>{project.availability}</small></button></li>{/each}</ul>{/if}
            {#if active.unresolvedProjects.length}<div class="notice warning"><p>Some folder creations need a decision. Their directories have been preserved.</p>{#each active.unresolvedProjects as operation (operation.request_id)}<button onclick={() => recoverProject(active!, operation)}>Recover folder creation {operation.operation_folder ?? operation.request_id}</button>{/each}</div>{/if}
          {/if}
          <div class="page-heading"><div><p class="eyebrow">{activeProject?.name ?? 'Retained work'}</p><h2>{activeProject ? 'Your workflows' : active.earlierOnly ? 'Earlier workflows' : 'All accessible workflows'}</h2><p>{activeProject ? `Work in progress and retained results in ${activeProject.name}.` : active.earlierOnly ? 'Work accepted before projects. Associate a workflow deliberately to use project settings for later tasks.' : `Permitted work and results on ${active.session.daemon_name}. Choose a project above for new work.`}</p></div>{#if activeProject}<button class="primary" onclick={() => openComposer(active!, { kind: 'new' })}>New workflow</button><button class="quiet" onclick={() => showProject(active!, activeProject)}>Project settings</button>{/if}</div>
          <form class="toolbar" onsubmit={(event) => { event.preventDefault(); search(active!); }}><label><span class="visually-hidden">Search workflows and retained results</span><input type="search" bind:value={active.search} placeholder="Search workflows and retained results" /></label><button>Search</button><button type="button" onclick={() => refresh(active!)}>Refresh</button></form>
          {#if active.workflows.length}
            {#if active.loading}<p role="status">Updating workflow results…</p>{/if}
            <ul class="workflow-list" aria-busy={active.loading}>{#each active.workflows as workflow (workflow.id)}<li><button class="workflow-card" disabled={active.loading} onclick={() => selectWorkflow(active!, workflow.id)}><span><h3>{workflow.title}</h3><p>{workflow.description}</p><small>{workflow.task_count} {workflow.task_count === 1 ? 'task' : 'tasks'} · Updated {timeLabel(workflow.updated_at)}</small></span><span><span class="badge {workflow.state}">{workflow.state}</span></span></button></li>{/each}</ul>
          {:else if active.loading}<p role="status">Loading workflows…</p>
          {:else if !active.error}
            <div class="panel empty"><div class="empty-symbol" aria-hidden="true">↗</div><h2>{active.appliedSearch ? 'No matching workflows' : activeProject ? 'Your first workflow starts with a task' : active.earlierOnly ? 'No earlier workflows' : 'No accessible workflows'}</h2><p>{active.appliedSearch ? 'The daemon returned no accessible workflows for this search. Try a different title or result phrase.' : activeProject ? 'Choose New workflow, describe the work, and follow its progress here. Add more tasks whenever you need them.' : 'Projects keep related workflows together around a folder you control.'}</p>{#if activeProject}<button class="primary" onclick={() => openComposer(active!, { kind: 'new' })}>Create a workflow</button>{/if}</div>
          {/if}
          {#if active.truncated}<p class="notice warning">The daemon’s bounded search window was exhausted. Refine the search to inspect more specific work.</p>{/if}
          {#if active.pageCursor || active.nextCursor}<div class="form-actions">{#if active.pageCursor}<button onclick={() => { active!.pageCursor = undefined; void refresh(active!); }}>First page</button>{/if}{#if active.nextCursor}<button onclick={() => refresh(active!, true)}>Next page</button>{/if}</div>{/if}
          {/if}
        {/if}
      {/key}
    {:else}
      <div class="page-heading"><div><p class="eyebrow">Your workspace</p><h1>Work continues here.</h1></div></div>
      <div class="panel empty"><div class="empty-symbol" aria-hidden="true">↗</div><h2>Connect to your work</h2><p>Choose a saved daemon or add a connection. Sign in with its scoped credential to discover your tasks and retained results.</p><button class="primary" onclick={addConnection}>Connect a daemon</button></div>
    {/if}
  </main>
</div>
