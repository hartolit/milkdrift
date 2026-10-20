<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import CanvasWorkspace from './CanvasWorkspace.svelte';
  import { type CanvasState } from '../lib/canvas';
  import { projectCanvas as createCanvas, project } from '../lib/canvas.test-fixtures';
  import type { ProjectDto, TaskSessionDto, WorkflowReadDto, WorkflowReceipt } from '../lib/protocol.generated';
  import type { DaemonClient } from '../lib/transport';
  let { session, client, workflow = null, currentProject = project, initial, onstate, onaccepted = async () => {}, onerror = () => {}, onmodels = () => {} }: {
    session: TaskSessionDto; client: DaemonClient; workflow?: WorkflowReadDto | null; initial?: CanvasState;
    currentProject?: ProjectDto;
    onstate?: (canvas: CanvasState) => void; onaccepted?: (receipt: WorkflowReceipt) => Promise<void>; onerror?: (failure: unknown) => void; onmodels?: () => void;
  } = $props();
  let canvas = $state(untrack(() => initial ?? createCanvas(session, workflow ? { kind: 'existing', workflowId: workflow.id, title: workflow.title, generation: workflow.generation } : { kind: 'new' })));
  onMount(() => onstate?.(canvas));
</script>
<CanvasWorkspace {canvas} {currentProject} {workflow} {session} {client} {onaccepted} {onerror} {onmodels} onreviewed={() => {}} onrefresh={async () => {}} onpage={async () => {}} oncapabilitychoice={() => {}} ondiscard={() => { canvas.tasks = []; canvas.sources = []; canvas.changes = []; canvas.inspector = false; }} />
