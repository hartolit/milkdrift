import type { InputBinding, TaskReadDto, TaskSummaryDto } from './protocol.generated';

export function inputLabel(input: InputBinding): string {
  switch (input.kind) {
    case 'inline': return input.value.kind === 'text' ? `Captured inline text: ${input.value.text}` : input.value.kind === 'uploaded' ? `Captured upload ${input.value.input_id}` : 'Explicit absence';
    case 'supplied': return `Workflow input ${input.name} · ${input.optional ? 'optional' : 'required'}`;
    case 'task_output': return `${input.task} / ${input.output} · ${input.optional ? 'optional after producer settles' : 'requires producer success'}`;
    case 'settled_outcome': return `Retained outcome of ${input.task}, including failure`;
  }
}

/** Shared actual-time coordinates; queued work has no invented placement on this axis. */
export function executionLanes(tasks: TaskSummaryDto[], observedAt: number): { task: TaskSummaryDto; left: number; width: number }[] {
  const started = tasks.filter((task) => task.started_at !== null);
  if (!started.length) return [];
  const origin = Math.min(...started.map((task) => task.started_at!));
  const end = Math.max(origin + 1, ...started.map((task) => task.ended_at ?? Math.max(observedAt, task.started_at!)));
  return started.map((task) => ({ task, left: 100 * (task.started_at! - origin) / (end - origin), width: 100 * (Math.max(task.started_at!, task.ended_at ?? observedAt) - task.started_at!) / (end - origin) }));
}

export function timeLabel(value: number | null): string {
  return value === null ? 'Not recorded' : new Date(value).toLocaleString(undefined, {
    month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit',
  });
}

export function stateLabel(task: Pick<TaskReadDto, 'state' | 'stop_requested'>): string {
  const label = task.state.charAt(0).toUpperCase() + task.state.slice(1);
  return task.stop_requested && task.state !== 'stopped' ? `${label} · stop requested` : label;
}

/** Dependency columns are presentation only; readiness always comes from the daemon. */
export function taskColumns(tasks: TaskSummaryDto[]): TaskSummaryDto[][] {
  const positions = new Map<string, number>();
  const remaining = new Set(tasks.map((task) => task.id));
  for (let pass = 0; pass < tasks.length && remaining.size; pass++) {
    let advanced = false;
    for (const task of tasks) {
      if (!remaining.has(task.id) || task.dependencies.some((id) => remaining.has(id))) continue;
      positions.set(task.id, Math.max(0, ...task.dependencies.map((id) => (positions.get(id) ?? -1) + 1)));
      remaining.delete(task.id);
      advanced = true;
    }
    if (!advanced) break;
  }
  const columns: TaskSummaryDto[][] = [];
  for (const task of tasks) {
    const column = positions.get(task.id) ?? 0;
    (columns[column] ??= []).push(task);
  }
  return columns;
}

export function saveBlob(name: string, blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  URL.revokeObjectURL(url);
}
