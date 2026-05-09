import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { fetchPolicy, fetchGovernanceEvents, reloadPolicy } from './governance';

const MOCK_POLICY = {
	id: 1,
	version: 3,
	state: 'active',
	policy: {
		global: {
			require_approval_for_mutations: false,
			allow_create_without_approval: true,
			allow_update: true,
			allow_delete: true,
			allow_publish: true
		},
		actions: {
			'mission.create': { enabled: true, requires_approval: false },
			'task.delete': { enabled: false, requires_approval: true }
		},
		terminal: { allow_create_actions: true, allow_publish_actions: false },
		mcp: { allow_mutation_tools: true }
	},
	change_note: 'initial',
	created_by: 'admin',
	published_by: 'admin',
	published_at: '2026-05-01T12:00:00Z',
	created_at: '2026-05-01T11:00:00Z',
	updated_at: '2026-05-01T12:00:00Z'
};

const MOCK_EVENTS = [
	{
		id: 1,
		policy_id: 1,
		version: 3,
		event_type: 'published',
		actor_subject: 'admin',
		detail: {},
		created_at: '2026-05-01T12:00:00Z'
	}
];

function mockFetch(body: unknown, status = 200) {
	return vi.fn().mockResolvedValue({
		ok: status >= 200 && status < 300,
		status,
		text: () => Promise.resolve(JSON.stringify(body))
	});
}

describe('fetchPolicy', () => {
	afterEach(() => vi.restoreAllMocks());

	it('calls /governance/policy/active and returns typed data', async () => {
		vi.stubGlobal('fetch', mockFetch(MOCK_POLICY));
		const result = await fetchPolicy('tok');
		expect(result.version).toBe(3);
		expect(result.state).toBe('active');
		expect(result.policy.global.allow_update).toBe(true);
		expect(result.policy.actions['mission.create'].enabled).toBe(true);
	});

	it('propagates auth token in Authorization header', async () => {
		const spy = mockFetch(MOCK_POLICY);
		vi.stubGlobal('fetch', spy);
		await fetchPolicy('mytoken');
		const headers = spy.mock.calls[0][1].headers as Headers;
		expect(headers.get('Authorization')).toBe('Bearer mytoken');
	});
});

describe('fetchGovernanceEvents', () => {
	afterEach(() => vi.restoreAllMocks());

	it('returns parsed events', async () => {
		vi.stubGlobal('fetch', mockFetch(MOCK_EVENTS));
		const events = await fetchGovernanceEvents(10, 'tok');
		expect(events).toHaveLength(1);
		expect(events[0].event_type).toBe('published');
		expect(events[0].actor_subject).toBe('admin');
	});

	it('uses the limit parameter in the URL', async () => {
		const spy = mockFetch(MOCK_EVENTS);
		vi.stubGlobal('fetch', spy);
		await fetchGovernanceEvents(25);
		expect(spy.mock.calls[0][0]).toContain('limit=25');
	});

	it('returns empty array on error', async () => {
		vi.stubGlobal('fetch', mockFetch({ error: 'forbidden' }, 403));
		const events = await fetchGovernanceEvents(10, 'bad');
		expect(events).toEqual([]);
	});
});

describe('reloadPolicy', () => {
	afterEach(() => vi.restoreAllMocks());

	it('sends POST to /governance/policy/reload', async () => {
		const spy = mockFetch({ ok: true });
		vi.stubGlobal('fetch', spy);
		const result = await reloadPolicy('tok');
		expect(spy.mock.calls[0][0]).toBe('/governance/policy/reload');
		expect(spy.mock.calls[0][1].method).toBe('POST');
		expect(result).toEqual({ ok: true });
	});
});
