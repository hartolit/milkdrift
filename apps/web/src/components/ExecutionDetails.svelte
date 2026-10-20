<script lang="ts">
  import type { ProjectDto, TaskDefinitionDto, TaskSessionDto } from '../lib/protocol.generated';
  let { definition, project, session }: { definition: TaskDefinitionDto; project: ProjectDto | null; session: TaskSessionDto } = $props();
  let execution = $derived(definition.execution);
  let binding = $derived(execution?.kind === 'bound' ? execution.binding : null);
  let settings = $derived(binding?.settings ?? (execution?.kind === 'explicit' ? execution.settings : project?.settings));
  let environment = $derived(settings?.environment);
  let environmentLabel = $derived(environment?.kind === 'host' ? 'Host file tools' : environment ? `${environment.kind === 'podman' ? 'Podman' : 'Host tools'} · ${session.authoring?.find((entry) => entry.capability.id === environment.capability && entry.capability.operation === environment.operation)?.label ?? environment.capability}` : 'Previously accepted tools');
  let access = $derived(binding?.access ?? (execution?.kind === 'explicit' ? execution.access : settings?.allow_changes ? 'write' : 'read'));
</script>

<div class="execution-details">
  <p><strong>Project:</strong> {project?.name ?? (execution?.kind === 'restricted' ? 'Restricted project' : binding ? 'Previously selected project' : 'Earlier workflow')}</p>
  {#if execution?.kind === 'restricted'}<p class="field-hint">The accepted execution details are restricted for this connection. Its project folder and tools are not disclosed.</p>
  {:else if !execution || execution.kind === 'legacy'}<p class="field-hint">This task keeps its earlier tools and paths. Current project settings do not replace them.</p>
  {:else}
    <p><strong>Execution environment:</strong> {access === 'none' && session.authoring?.find((entry) => entry.capability.id === definition.capability.id)?.category === 'process' ? 'Isolated process · no project files' : environmentLabel}</p>
    <p><strong>Project access:</strong> {access === 'write' ? 'Read and write' : access === 'read' ? 'Read only' : 'No project access'}{execution.kind === 'explicit' ? ' · task override' : execution.kind === 'bound' ? ' · accepted selection' : ' · inherited'}</p>
    {#if binding && project && binding.generation !== project.generation}<p class="field-hint">Project settings changed after this task was accepted. This task keeps generation {binding.generation}.</p>{/if}
    {#if environment?.kind === 'process'}<p class="field-hint">Trusted host tools run on {session.daemon_name}. They are not sandboxed by file claims.</p>{:else if environment?.kind === 'podman'}<p class="field-hint">Tools use the attached folder at /project inside the worker.</p>{/if}
  {/if}
</div>
