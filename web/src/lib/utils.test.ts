import { describe, it, expect } from 'vitest';
import { cn } from './utils';
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
