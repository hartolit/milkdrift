<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import ProjectSettings, { newProjectDraft, type ProjectDraft } from './ProjectSettings.svelte';
  import type { ProjectDto, ProjectReceiptDto, TaskSessionDto } from '../lib/protocol.generated';
  import type { DaemonClient } from '../lib/transport';
  let { client, session, project = null, initial, onstate, onaccepted = async () => {}, onrefresh = async () => {} }: {
    client: DaemonClient; session: TaskSessionDto; project?: ProjectDto | null; initial?: ProjectDraft;
    onstate?: (draft: ProjectDraft) => void; onaccepted?: (receipt: ProjectReceiptDto) => Promise<void>; onrefresh?: () => Promise<void>;
  } = $props();
  let draft = $state(untrack(() => initial ?? newProjectDraft(project?.id ?? null)));
  onMount(() => onstate?.(draft));
</script>
<ProjectSettings {client} {session} {project} bind:draft {onaccepted} {onrefresh} onerror={() => {}} onback={() => {}} onmodels={() => {}} />
