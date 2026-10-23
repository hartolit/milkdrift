import { expect, it } from 'vitest';
import { executionLanes, readableReason, stateLabel } from './presentation';
import type { TaskSummaryDto } from './protocol.generated';

function task(id: string, start: number | null, end: number | null): TaskSummaryDto {
  return { id, version: 1, title: id, description: '', dependencies: [], state: start === null ? 'queued' : end === null ? 'running' : 'completed', reason: null, started_at: start, ended_at: end, entered: start !== null, stop_requested: false, stop_request_id: `stop-${id}`, has_result: end !== null };
}

it('plots recorded start-to-settlement intervals and excludes work without a frozen start', () => {
  const lanes = executionLanes([task('first', 100, 400), task('second', 200, 300), task('live', 350, null), task('queued', null, null)], 500);
  expect(lanes.map(({ task, left, width }) => [task.id, left, width])).toEqual([['first', 0, 75], ['second', 25, 25], ['live', 62.5, 37.5]]);
  expect(executionLanes([task('queued', null, null)], 500)).toEqual([]);
});

it('distinguishes frozen preparation and deferred entry from running or proven stop', () => {
  const frozen = { ...task('frozen', 10, null), entered: false };
  expect(stateLabel(frozen)).toBe('Preparing');
  expect(stateLabel({ ...frozen, state: 'queued' })).toBe('Waiting before entry');
  expect(stateLabel({ ...frozen, entered: true })).toBe('Running');
  expect(stateLabel({ ...frozen, state: 'uncertain', stop_requested: true })).toBe('Uncertain · stop requested');
  expect(stateLabel({ ...frozen, state: 'stopped', stop_requested: true })).toBe('Stopped');
  expect(readableReason('Project is in use')).toBe('Project is in use');
});

it('presents required-input waiting without generated identities and preserves unknown diagnostics', () => {
  expect(readableReason('missing supplied input source_generated_identity')).toBe('Waiting for a required input');
  expect(readableReason('process exited with code 3')).toBe('process exited with code 3');
  expect(readableReason(null)).toBeNull();
});
