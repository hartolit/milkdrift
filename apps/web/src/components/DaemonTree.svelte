<script lang="ts">
  import { untrack } from 'svelte';
  import { SvelteMap, SvelteURLSearchParams } from 'svelte/reactivity';
  import type { ConnectionProfile } from '../lib/connections';
  import type { ProjectDto, TaskSessionDto, WorkflowPageDto, WorkflowSummaryDto } from '../lib/protocol.generated';
  import type { DaemonClient } from '../lib/transport';
  interface Branch { profile: ConnectionProfile; client: DaemonClient; session: TaskSessionDto; projects: ProjectDto[]; projectCursor: string | null; selectedId: string; selected?: { id: string; title: string; project_id: string | null; task_count: number; state: string } | null; }
  let { profiles, connections, activeId, onchoose, onselect, onnew, onproject, onmoreprojects, onsettings, onmodels, onerror }: {
    profiles: ConnectionProfile[]; connections: Branch[]; activeId: string;
    onchoose: (profile: ConnectionProfile) => void;
    onselect: (id: string, workflow: string) => void;
    onnew: (id: string, project: ProjectDto) => void;
    onproject: (id: string, project: ProjectDto | null) => void;
    onmoreprojects: (id: string) => void;
    onsettings: (id: string) => void; onmodels: (id: string) => void;
    onerror: (id: string, failure: unknown) => void;
  } = $props();
  const storageKey = 'milkdrift.tree.v1';
  function restored(): string[] { try { const value: unknown = JSON.parse(localStorage.getItem(storageKey) ?? '[]'); return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string').slice(-600) : []; } catch { return []; } }
  let expanded = $state<string[]>(untrack(restored));
  let pages = $state<Record<string, { items: WorkflowSummaryDto[]; next: string | null; loading: boolean; owner: DaemonClient; request: number }>>({});
  const selectedSignatures = new SvelteMap<string, string>();
  let requestSequence = 0;
  let searches = $state<Record<string, string>>({});
  const searchOwners = new SvelteMap<string, DaemonClient>();
  const key = (branch: Branch, project: string) => `${branch.session.daemon_id}/${branch.session.actor_id}/${project}`;
  function visiblePage(branch: Branch, project: string) { const page = pages[key(branch, project)]; return page?.owner === branch.client ? page : undefined; }
  function toggle(id: string) { expanded = expanded.includes(id) ? expanded.filter((item) => item !== id) : [...expanded, id].slice(-600); try { localStorage.setItem(storageKey, JSON.stringify(expanded)); } catch { /* Expansion remains usable when preference storage is unavailable. */ } }
  async function load(branch: Branch, project: string, next = false, search = '') {
    const id = key(branch, project), previous = pages[id]?.owner === branch.client ? pages[id] : undefined;
    const request = ++requestSequence;
    // Only visible pages are cached. Returning to a branch re-reads the daemon's bounded page.
    if (!previous && Object.keys(pages).length >= 16) { const oldest = Object.keys(pages).find((item) => !expanded.includes(item)); if (oldest) delete pages[oldest]; else { onerror(branch.profile.id, new Error('Close another project branch before loading more workflow pages. The canvas is unchanged.')); return; } }
    pages[id] = { items: previous?.items ?? [], next: previous?.next ?? null, loading: true, owner: branch.client, request };
    const query = new SvelteURLSearchParams({ limit: '30', search });
    if (project) query.set('project_id', project);
    if (next && previous?.next) query.set('cursor', previous.next);
    try {
      const result = await branch.client.request<WorkflowPageDto>(`/v1/workflows?${query}`);
      if (!connections.some((item) => item.client === branch.client && item.session.actor_id === branch.session.actor_id) || pages[id]?.request !== request) return;
      pages[id] = { items: result.items, next: result.next_cursor, loading: false, owner: branch.client, request };
    } catch (failure) { if (pages[id]?.request === request && connections.some((item) => item.client === branch.client)) { pages[id].loading = false; onerror(branch.profile.id, failure); } }
  }
  $effect(() => {
    const current = connections.map((branch) => ({ client: branch.client, prefixes: [key(branch, ''), ...branch.projects.map((project) => key(branch, project.id))] }));
    const stale = Object.entries(pages).filter(([id, page]) => !current.some((owner) => owner.client === page.owner && owner.prefixes.includes(id))).map(([id]) => id);
    untrack(() => {
      for (const id of stale) { delete pages[id]; selectedSignatures.delete(id); }
      for (const [id, client] of searchOwners) if (!connections.some((branch) => branch.profile.id === id && branch.client === client)) { delete searches[id]; searchOwners.delete(id); }
      for (const branch of connections) searchOwners.set(branch.profile.id, branch.client);
    });
  });
  $effect(() => {
    const changes = connections.flatMap((branch) => {
      const selected = branch.selected, id = selected?.project_id ? key(branch, selected.project_id) : '';
      const signature = selected ? `${selected.id}:${selected.title}:${selected.task_count}:${selected.state}` : '';
      return selected && id && expanded.includes(id) && selectedSignatures.get(id) !== signature ? [{ branch, project: selected.project_id!, id, signature }] : [];
    });
    untrack(() => { for (const change of changes) { selectedSignatures.set(change.id, change.signature); void load(change.branch, change.project); } });
  });
  $effect(() => {
    const requests = connections.flatMap((branch) => branch.projects.filter((project) => expanded.includes(key(branch, project.id)) && !pages[key(branch, project.id)]).map((project) => ({ branch, project }))).slice(0, Math.max(0, 16 - Object.keys(pages).length));
    untrack(() => { for (const { branch, project } of requests) void load(branch, project.id); });
  });
</script>

<nav class="daemon-tree" aria-label="Daemons, projects and workflows">
  {#each profiles as profile (profile.id)}
    {@const branch = connections.find((item) => item.profile.id === profile.id)}
    {@const daemonKey = branch ? key(branch, '') : profile.daemonId}
    <div class="tree-row daemon-row">
      <button class="tree-expand" aria-label={`${expanded.includes(daemonKey) ? 'Collapse' : 'Expand'} ${profile.cachedName}`} aria-expanded={expanded.includes(daemonKey)} onclick={() => { if (branch) toggle(daemonKey); else onchoose(profile); }}>{expanded.includes(daemonKey) ? '▾' : '▸'}</button>
      <button class="tree-label" class:active={activeId === profile.id} onclick={() => onchoose(profile)}><span class="status-dot" class:connected={!!branch} aria-hidden="true"></span>{branch?.session.daemon_name ?? profile.cachedName}{#if profiles.filter((item) => item.cachedName === profile.cachedName).length > 1}<small>{profile.endpoint}</small>{/if}{#if !branch}<small>Reconnect</small>{/if}</button>
      {#if branch}<button class="quiet" aria-label={`Settings for ${profile.cachedName}`} onclick={() => onsettings(profile.id)}>⚙</button>{/if}
    </div>
    {#if branch && expanded.includes(daemonKey)}
      <div class="tree-children">
        <form class="tree-search" onsubmit={(event) => { event.preventDefault(); void load(branch, '', false, searches[profile.id] ?? ''); }}><label><span class="visually-hidden">Search {profile.cachedName}</span><input type="search" placeholder="Find a workflow" value={searchOwners.get(profile.id) === branch.client ? searches[profile.id] ?? '' : ''} oninput={(event) => { searchOwners.set(profile.id, branch.client); searches[profile.id] = event.currentTarget.value; }} /></label><button aria-label={`Search ${profile.cachedName} workflows`}>⌕</button></form>
        {#if visiblePage(branch, '')}<div class="tree-results" aria-label={`${profile.cachedName} search results`}>{#each visiblePage(branch, '')!.items as workflow (workflow.id)}<button onclick={() => onselect(profile.id, workflow.id)}><span>{workflow.title}</span><small>{branch.projects.find((project) => project.id === workflow.project_id)?.name ?? 'Earlier workflow'}</small></button>{:else}<p class="field-hint">No matching workflows.</p>{/each}{#if visiblePage(branch, '')!.next}<button onclick={() => load(branch, '', true, searches[profile.id] ?? '')}>Next results</button>{/if}</div>{/if}
        {#each branch.projects as project (project.id)}
          {@const projectKey = key(branch, project.id)}
          <div class="tree-row project-row"><button class="tree-expand" aria-label={`${expanded.includes(projectKey) ? 'Collapse' : 'Expand'} project ${project.name}`} aria-expanded={expanded.includes(projectKey)} onclick={() => { toggle(projectKey); if (expanded.includes(projectKey)) void load(branch, project.id); }}>{expanded.includes(projectKey) ? '▾' : '▸'}</button><span class="tree-project-name">{project.name}</span><button class="quiet" aria-label={`Settings for ${project.name}`} onclick={() => onproject(profile.id, project)}>⚙</button></div>
          {#if expanded.includes(projectKey)}<div class="tree-workflows"><button class="tree-new" aria-label={`New workflow in ${project.name}`} onclick={() => onnew(profile.id, project)}>+ New workflow</button>{#each visiblePage(branch, project.id)?.items ?? [] as workflow (workflow.id)}<button class:active={activeId === profile.id && branch.selectedId === workflow.id} aria-current={activeId === profile.id && branch.selectedId === workflow.id ? 'page' : undefined} onclick={() => onselect(profile.id, workflow.id)}>{workflow.title}<small>{workflow.task_count} tasks · {workflow.state}</small></button>{/each}{#if visiblePage(branch, project.id)?.loading}<p class="field-hint">Loading…</p>{/if}{#if visiblePage(branch, project.id)?.next}<button class="quiet" onclick={() => load(branch, project.id, true)}>Next workflows</button>{/if}<button class="quiet tree-refresh" onclick={() => load(branch, project.id)}>Refresh workflows</button></div>{/if}
        {/each}
        {#if branch.projectCursor}<button class="quiet" onclick={() => onmoreprojects(profile.id)}>More projects</button>{/if}
        <button class="quiet" onclick={() => onproject(profile.id, null)}>+ Add project</button>
        <button class="quiet" onclick={() => onmodels(profile.id)}>Models & agents</button>
      </div>
    {/if}
  {/each}
</nav>

<style>
  .daemon-tree { margin-bottom: 20px; }
  .tree-row { display: flex; align-items: center; gap: 2px; min-width: 0; }
  .tree-row button { border: 0; background: transparent; min-height: 40px; padding: 6px; }
  .tree-label { flex: 1; text-align: left; min-width: 0; overflow-wrap: anywhere; }
  .tree-label.active, .tree-workflows button.active { color: #ead8ff; background: #342a454d; }
  .tree-expand { width: 30px; flex-shrink: 0; }
  .tree-children { margin: 4px 0 14px 12px; border-left: 1px solid #33354b; padding-left: 8px; }
  .tree-project-name { flex: 1; min-width: 0; overflow-wrap: anywhere; font-size: .8rem; }
  .tree-workflows { margin-left: 20px; display: grid; gap: 3px; }
  .tree-workflows button, .tree-results button { text-align: left; border: 0; background: transparent; font-size: .76rem; min-height: 40px; }
  .tree-workflows small, .tree-label small { display: block; color: var(--muted); font-size: .65rem; }
  .tree-new { color: #c5a5ed; }
  .tree-search { display: flex; gap: 3px; margin-bottom: 8px; }
  .tree-search label { min-width: 0; margin: 0; flex: 1; }
  .tree-search input { min-width: 0; width: 100%; padding: 7px; font-size: .75rem; }
  .tree-search button { padding: 6px; }
  .tree-refresh { font-size: .65rem !important; color: var(--muted); }
</style>
