<script lang="ts">
  import type { TaskDefinitionDto, TaskSessionDto } from '../lib/protocol.generated';
  import { capabilityKey, copy } from '../lib/canvas';
  let { definition, session, onchange, onerror }: { definition: TaskDefinitionDto; session: TaskSessionDto; onchange: (definition: TaskDefinitionDto) => void; onerror: (message: string) => void } = $props();
  let root = $state('');
  let folder = $state('');
  let access = $state('read');
  let process = $state('');
  function change(update: (next: TaskDefinitionDto) => void) { const next = copy(definition); update(next); onchange(next); }
  function addFolder() {
    const selected = session.project_roots?.find((item) => item.path === root);
    const parts = folder.split(/[\\/]/);
    if (!selected || parts.some((part) => part === '..' || part === '.') || folder.startsWith('/') || folder.includes('\\')) { onerror('Choose an available project and a relative subfolder without parent-directory navigation.'); return; }
    if (access === 'write' && !selected.write || access === 'read' && !selected.read) { onerror('That access is not available for this project.'); return; }
    if (definition.paths.length >= 32) { onerror('This task has reached its limit of 32 project folders.'); return; }
    const additions = Number(selected.read && !definition.tools.some((tool) => tool.kind === 'read_file')) + Number(access === 'write' && !definition.tools.some((tool) => tool.kind === 'write_file'));
    if (definition.tools.length + additions > 32) { onerror('Remove a tool before enabling more project access; this task already uses its tool allowance.'); return; }
    const path = folder ? `${root.replace(/\/$/, '')}/${parts.filter(Boolean).join('/')}` : root;
    change((definition) => {
    definition.paths = [...definition.paths.filter((entry) => entry.path !== path), { path, write: access === 'write' }];
    if (selected.read && !definition.tools.some((tool) => tool.kind === 'read_file')) definition.tools = [...definition.tools, { kind: 'read_file' }];
    if (access === 'write' && !definition.tools.some((tool) => tool.kind === 'write_file')) definition.tools = [...definition.tools, { kind: 'write_file' }];
    });
    folder = '';
  }
  function removeFolder(path: string) {
    change((definition) => {
    definition.paths = definition.paths.filter((entry) => entry.path !== path);
    if (!definition.paths.length) definition.tools = definition.tools.filter((tool) => tool.kind !== 'read_file' && tool.kind !== 'write_file');
    else if (!definition.paths.some((claim) => claim.write)) definition.tools = definition.tools.filter((tool) => tool.kind !== 'write_file');
    if (!definition.paths.some((claim) => session.project_roots.some((root) => root.read && (claim.path === root.path || root.path === '/' || claim.path.startsWith(`${root.path}/`))))) definition.tools = definition.tools.filter((tool) => tool.kind !== 'read_file');
    });
  }
</script>

<details class="task-permissions"><summary>Tools & autonomy</summary>
  <label class="checkbox-label"><input type="checkbox" checked={definition.actions !== null} onchange={(event) => change((next) => { next.actions = event.currentTarget.checked ? { automatic_repair: false } : null; })} /> Let this model propose follow-up tasks</label>
  {#if definition.actions}<label class="checkbox-label"><input type="checkbox" checked={definition.actions.automatic_repair} onchange={(event) => change((next) => { if (next.actions) next.actions.automatic_repair = event.currentTarget.checked; })} /> Let it retry refused proposals within this workflow’s limits</label>{/if}
  <details><summary>Project files</summary><p class="field-hint">Captured input files need no project access. These permissions let the model use files on the daemon.</p>
    {#each definition.paths as claim (claim.path)}<div class="permission-row"><span>{session.project_roots?.find((item) => claim.path === item.path || claim.path.startsWith(`${item.path}/`))?.label ?? 'Previously selected folder'}<small>{claim.write ? 'Writing permitted' : 'Read only'} · {claim.path}</small></span><button type="button" class="quiet" onclick={() => removeFolder(claim.path)}>Remove</button></div>{/each}
    {#if session.project_roots?.length}
      <label>Project<select bind:value={root} onchange={() => access = session.project_roots.find((item) => item.path === root)?.read ? 'read' : 'write'}><option value="">Choose an available project</option>{#each session.project_roots as project (project.path)}<option value={project.path}>{project.label}{session.project_roots.filter((item) => item.label === project.label).length > 1 ? ` · ${project.path}` : ''}</option>{/each}</select></label>
      {#if root}<label>Subfolder <span class="field-hint">Optional, inside this project</span><input bind:value={folder} maxlength="256" placeholder="Whole project" /></label><label>Access<select bind:value={access}>{#if session.project_roots.find((item) => item.path === root)?.read}<option value="read">Read only</option>{/if}{#if session.project_roots.find((item) => item.path === root)?.write}<option value="write">{session.project_roots.find((item) => item.path === root)?.read ? 'Read & write' : 'Write only'}</option>{/if}</select></label><button type="button" onclick={addFolder}>Allow project access</button>{/if}
    {:else}<p class="field-hint">No project folders are available with this connection’s permissions.</p>{/if}
  </details>
  <details><summary>Other tools</summary>
    {#each definition.tools.filter((tool) => tool.kind === 'process') as tool (capabilityKey(tool.capability))}<div class="permission-row"><span>{session.authoring?.find((item) => capabilityKey(item.capability) === capabilityKey(tool.capability))?.label ?? tool.capability.id}</span><button class="quiet" type="button" onclick={() => change((next) => { next.tools = next.tools.filter((entry) => entry.kind !== 'process' || capabilityKey(entry.capability) !== capabilityKey(tool.capability)); })}>Remove</button></div>{/each}
    <label>Available tool<select bind:value={process}><option value="">Choose a tool</option>{#each session.authoring?.filter((item) => item.category === 'process') ?? [] as tool (capabilityKey(tool.capability))}<option value={capabilityKey(tool.capability)}>{tool.label}</option>{/each}</select></label><button type="button" disabled={!process || definition.tools.length >= 32} onclick={() => { const tool = session.authoring.find((item) => capabilityKey(item.capability) === process); if (tool && !definition.tools.some((entry) => entry.kind === 'process' && capabilityKey(entry.capability) === process)) change((next) => { next.tools = [...next.tools, { kind: 'process', capability: { ...tool.capability } }]; }); process = ''; }}>Allow tool</button>
  </details>
</details>
