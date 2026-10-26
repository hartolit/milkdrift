import type { ProjectDto, TaskReadDto, TaskSessionDto, WorkflowReadDto, WorkflowTopologyDto } from './protocol.generated';
import { addCanvasTask, createCanvas, dependencyIds, updateInstructions, type CanvasTarget } from './canvas';

export const project: ProjectDto = { id: 'project-fixture', name: 'Fixture project', generation: 1, folder: '/tmp/fixture-project', settings: { environment: { kind: 'host' }, allow_changes: false }, availability: 'available', can_manage: true };
export function projectCanvas(configured: TaskSessionDto, target: CanvasTarget) {
  return createCanvas(configured, { ...target, projectId: project.id }, project);
}

export function session(): TaskSessionDto {
  const capability = { id: 'model-a', operation: 'model.generate' };
  return { authoring_bounds: { document_bytes: 2097152, pending_tasks: 200, sources: 400, connections: 1024, positions: 600, run_actions: 32 }, daemon_id: 'daemon-a', daemon_name: 'Studio A', daemon_name_generation: 1, can_rename_daemon: true, actor_id: 'actor-a', input_limit_bytes: 16384, task_output_limit_bytes: 2097152, capabilities: [capability], project_roots: [], limits: { tasks: 128, concurrent_tasks: 8, model_batches: 16, rejected_responses: 4, corrective_failures: 4, active_execution_ms: 600000, tool_calls_per_task: 16 }, authoring: [{ capability, category: 'model', input_media: 'text_json_or_image', generation: 3, label: 'Research model', inputs: [], allows_additional_inputs: true, outputs: [{ name: 'model_response', label: 'Complete response', required: true, media_type: 'application/json', max_bytes: 1048576 }, { name: 'final_text', label: 'Answer text', required: false, media_type: 'text/plain', max_bytes: 1048576 }], default_output: 'model_response' }] };
}
export function acceptedTask(id = 'accepted-task'): TaskReadDto {
  const canvas = projectCanvas(session(), { kind: 'new' });
  const draft = addCanvasTask(canvas, session(), session().capabilities[0]);
  updateInstructions(canvas, draft, 'Accepted task');
  return { id, version: 1, definition: draft.definition, state: 'queued', reason: null, started_at: null, ended_at: null, stop_requested: false, stop_request_id: `stop-${id}`, frozen: null, result: null, result_restricted: false, origin: null, inline_inputs: {}, supplied_inputs: {} };
}
export function workflow(tasks: TaskReadDto[] = []): WorkflowReadDto {
  return { id: 'workflow-a', project_id: project.id, title: 'Research workflow', description: '', generation: 5, state: 'waiting', created_at: 1, updated_at: 2, limits: session().limits, tasks: tasks.map((task) => ({ id: task.id, title: task.definition.title, description: task.definition.description, dependencies: dependencyIds(task.definition.dependencies), state: task.state, version: task.version, reason: task.reason, started_at: task.started_at, ended_at: task.ended_at, entered: task.frozen?.entered === true, stop_requested: task.stop_requested, stop_request_id: task.stop_request_id, has_result: !!task.result })), task_count: tasks.length, next_task_cursor: null, supplied_inputs: {}, wait_question: null, admitted_tasks: tasks.length, admitted_model_batches: 0, rejected_responses: 0, active_execution_ms: 0, admissions: [] };
}

export function topology(tasks: TaskReadDto[] = []): WorkflowTopologyDto {
  return { workflow_generation: 5, tasks: tasks.map((task) => ({ id: task.id, version: task.version, capability: task.definition.capability, inputs: {}, outputs: task.definition.outputs, published_outputs: task.result?.outputs ?? {}, dependencies: Array.isArray(task.definition.dependencies) ? Object.fromEntries(task.definition.dependencies.map((id) => [id, 'success' as const])) : task.definition.dependencies, dependencies_legacy: Array.isArray(task.definition.dependencies) })), next_cursor: null, truncated: false };
}
