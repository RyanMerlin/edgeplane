import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { cn, statusClass, fmtRelative, taskCountByStatus } from './utils';
import { ApiError } from './api/client';

describe('cn', () => {
	it('merges class names', () => {
		expect(cn('a', 'b')).toBe('a b');
	});

	it('deduplicates conflicting tailwind classes', () => {
		expect(cn('px-2', 'px-4')).toBe('px-4');
	});

	it('handles conditional falsy values', () => {
		expect(cn('base', false && 'skipped', null, undefined, 'end')).toBe('base end');
	});
});

describe('ApiError', () => {
	it('sets name, message, status, and body', () => {
		const err = new ApiError('not found', 404, { error: 'missing' });
		expect(err.name).toBe('ApiError');
		expect(err.message).toBe('not found');
		expect(err.status).toBe(404);
		expect(err.body).toEqual({ error: 'missing' });
	});

	it('is an instance of Error', () => {
		expect(new ApiError('x', 500, null)).toBeInstanceOf(Error);
	});
});

describe('statusClass', () => {
	it('returns status-done for done/completed', () => {
		expect(statusClass('done')).toBe('status-done');
		expect(statusClass('completed')).toBe('status-done');
		expect(statusClass('DONE')).toBe('status-done');
	});

	it('returns status-blocked for blocked/failed/error', () => {
		expect(statusClass('blocked')).toBe('status-blocked');
		expect(statusClass('failed')).toBe('status-blocked');
		expect(statusClass('error')).toBe('status-blocked');
	});

	it('returns status-progress for in_progress/running', () => {
		expect(statusClass('in_progress')).toBe('status-progress');
		expect(statusClass('running')).toBe('status-progress');
	});

	it('returns status-proposed as fallback', () => {
		expect(statusClass('proposed')).toBe('status-proposed');
		expect(statusClass('unknown')).toBe('status-proposed');
		expect(statusClass(null)).toBe('status-proposed');
		expect(statusClass(undefined)).toBe('status-proposed');
	});
});

describe('fmtRelative', () => {
	let now: number;

	beforeEach(() => {
		now = Date.now();
		vi.spyOn(Date, 'now').mockReturnValue(now);
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('returns "just now" for sub-minute timestamps', () => {
		expect(fmtRelative(now - 30_000)).toBe('just now');
	});

	it('returns minutes ago for recent timestamps', () => {
		expect(fmtRelative(now - 5 * 60_000)).toBe('5m ago');
	});

	it('returns hours ago', () => {
		expect(fmtRelative(now - 3 * 3_600_000)).toBe('3h ago');
	});

	it('returns days ago', () => {
		expect(fmtRelative(now - 2 * 86_400_000)).toBe('2d ago');
	});

	it('accepts ISO string', () => {
		expect(fmtRelative(new Date(now - 10 * 60_000).toISOString())).toBe('10m ago');
	});
});

describe('taskCountByStatus', () => {
	const tasks = [
		{ status: 'done' },
		{ status: 'done' },
		{ status: 'in_progress' },
		{ status: 'blocked' },
		{}
	];

	it('counts matching status case-insensitively', () => {
		expect(taskCountByStatus(tasks, 'done')).toBe(2);
		expect(taskCountByStatus(tasks, 'DONE')).toBe(2);
	});

	it('counts in_progress', () => {
		expect(taskCountByStatus(tasks, 'in_progress')).toBe(1);
	});

	it('returns 0 for unmatched status', () => {
		expect(taskCountByStatus(tasks, 'proposed')).toBe(0);
	});

	it('handles tasks with no status field', () => {
		expect(taskCountByStatus(tasks, '')).toBe(1);
	});
});
