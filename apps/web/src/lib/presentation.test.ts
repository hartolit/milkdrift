import { expect, it } from 'vitest';
import { executionLanes, inputLabel } from './presentation';
import type { TaskSummaryDto } from './protocol.generated';

function task(id: string, start: number | null, end: number | null): TaskSummaryDto {
  return { id, version: 1, title: id, description: '', dependencies: [], state: start === null ? 'queued' : end === null ? 'running' : 'completed', reason: null, started_at: start, ended_at: end, stop_requested: false, stop_request_id: `stop-${id}`, has_result: end !== null };
}

it('plots recorded overlapping intervals on the same axis and excludes queued work', () => {
  const lanes = executionLanes([task('first', 100, 400), task('second', 200, 300), task('live', 350, null), task('queued', null, null)], 500);
  expect(lanes.map(({ task, left, width }) => [task.id, left, width])).toEqual([['first', 0, 75], ['second', 25, 25], ['live', 62.5, 37.5]]);
  expect(executionLanes([task('queued', null, null)], 500)).toEqual([]);
});

it('keeps input success and settlement policies readable and distinct', () => {
  expect(inputLabel({ kind: 'task_output', task: 'source', output: 'notes', optional: false })).toContain('requires producer success');
  expect(inputLabel({ kind: 'task_output', task: 'source', output: 'notes', optional: true })).toContain('optional after producer settles');
  expect(inputLabel({ kind: 'settled_outcome', task: 'source' })).toContain('including failure');
});
