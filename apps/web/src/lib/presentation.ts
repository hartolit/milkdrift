import type { TaskReadDto, TaskSummaryDto } from './protocol.generated';

/** Generated input identities belong in diagnostics, not the ordinary waiting message. */
export function readableReason(reason: string | null): string | null {
  return reason?.startsWith('missing supplied input ') ? 'Waiting for a required input' : reason;
}

/** Recorded freeze-to-settlement intervals include preparation and deferred entry. */
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

export function stateLabel(task: Pick<TaskReadDto, 'state' | 'stop_requested' | 'started_at' | 'frozen'> | Pick<TaskSummaryDto, 'state' | 'stop_requested' | 'started_at' | 'entered'>): string {
  const entered = 'entered' in task ? task.entered : task.frozen?.entered === true;
  const label = task.state === 'running' && !entered ? 'Preparing'
    : task.state === 'queued' && task.started_at !== null ? 'Waiting before entry'
    : task.state.charAt(0).toUpperCase() + task.state.slice(1);
  return task.stop_requested && task.state !== 'stopped' ? `${label} · stop requested` : label;
}

export function saveBlob(name: string, blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  URL.revokeObjectURL(url);
}
