<script lang="ts">
  import type { ProjectDto, TaskDefinitionDto, TaskSessionDto } from '../lib/protocol.generated';
  import { authoring, copy } from '../lib/canvas';
  import ExecutionDetails from './ExecutionDetails.svelte';
  let { definition, session, project, onchange }: {
    definition: TaskDefinitionDto; session: TaskSessionDto; project: ProjectDto | null;
    onchange: (definition: TaskDefinitionDto) => void; onerror: (message: string) => void;
  } = $props();
  const accessMode = $derived.by(() => {
    const selection = definition.execution;
    if (selection?.kind === 'project') return 'project';
    if (selection?.kind === 'explicit' && selection.settings.environment.kind === 'host' && !selection.settings.allow_changes && definition.paths.length === 0) {
      if (selection.access === 'none' && selection.tools.length === 0) return 'none';
      if (selection.access === 'read' && selection.tools.length === 1 && selection.tools[0].kind === 'read_file') return 'read';
    }
    return 'accepted';
  });
  function change(update: (next: TaskDefinitionDto) => void) { const next = copy(definition); update(next); onchange(next); }
  function permissions(mode: string) {
    if (!project) return;
    change((next) => {
      next.paths = []; next.tools = [];
      next.execution = mode === 'project'
        ? { kind: 'project', expected_generation: project.generation }
        : { kind: 'explicit', expected_generation: project.generation, settings: { environment: { kind: 'host' }, allow_changes: false }, access: mode === 'read' ? 'read' : 'none', tools: mode === 'read' ? [{ kind: 'read_file' }] : [] };
    });
  }
</script>

<details class="task-permissions"><summary>Tools & autonomy</summary>
  {#if authoring(session, definition.capability)?.category === 'model'}<label class="checkbox-label"><input type="checkbox" checked={definition.actions !== null} onchange={(event) => change((next) => { next.actions = event.currentTarget.checked ? { automatic_repair: false } : null; })} /> Let this model propose follow-up tasks</label>{/if}
  {#if definition.actions}<label class="checkbox-label"><input type="checkbox" checked={definition.actions.automatic_repair} onchange={(event) => change((next) => { if (next.actions) next.actions.automatic_repair = event.currentTarget.checked; })} /> Let it retry refused proposals within this workflow’s limits</label>{/if}
  <ExecutionDetails {definition} {project} {session} />
  {#if project}
    <label>Task project access<select value={accessMode} onchange={(event) => permissions(event.currentTarget.value)}>
      {#if accessMode === 'accepted'}<option value="accepted">{definition.execution?.kind === 'explicit' ? 'Keep reviewed task selection' : 'Keep accepted selection'}</option>{/if}
      <option value="project">Use reviewed project settings</option><option value="none">No project tools or file access</option><option value="read">Read files only</option>
    </select></label>
    {#if authoring(session, definition.capability)?.category === 'process'}<p class="field-hint">An isolated captured-input process requires “No project tools or file access”. Project work requires a compatible configured environment; the daemon refuses mismatches.</p>{/if}
    <p class="field-hint">A task override is deliberate. Empty permissions remain empty when project defaults change.</p>
  {:else}<p class="field-hint">Associate this earlier workflow with a project before adding work. Accepted tasks retain their original selection.</p>{/if}
</details>
