import { request, authHeader } from './client';

export type PolicyActionRule = {
	enabled: boolean;
	requires_approval: boolean;
};

export type PolicyGlobal = {
	require_approval_for_mutations: boolean;
	allow_create_without_approval: boolean;
	allow_update: boolean;
	allow_delete: boolean;
	allow_publish: boolean;
};

export type PolicyDoc = {
	global: PolicyGlobal;
	actions: Record<string, PolicyActionRule>;
	terminal: { allow_create_actions: boolean; allow_publish_actions: boolean };
	mcp: { allow_mutation_tools: boolean };
};

export type PolicyRecord = {
	id: number;
	version: number;
	state: 'active' | 'draft' | 'archived';
	policy: PolicyDoc;
	change_note: string;
	created_by: string;
	published_by: string;
	published_at: string | null;
	created_at: string;
	updated_at: string;
};

export type PolicyEvent = {
	id: number;
	policy_id: number | null;
	version: number;
	event_type: string;
	actor_subject: string;
	detail: Record<string, unknown>;
	created_at: string;
};

export function fetchPolicy(token?: string) {
	return request<PolicyRecord>('/governance/policy/active', { headers: authHeader(token) });
}

export function fetchPolicyVersions(token?: string) {
	return request<PolicyRecord[]>('/governance/policy/versions', { headers: authHeader(token) });
}

export async function fetchGovernanceEvents(limit = 50, token?: string): Promise<PolicyEvent[]> {
	try {
		return await request<PolicyEvent[]>(`/governance/policy/events?limit=${limit}`, {
			headers: authHeader(token)
		});
	} catch {
		return [];
	}
}

export function reloadPolicy(token?: string) {
	return request<{ ok: boolean }>('/governance/policy/reload', {
		method: 'POST',
		headers: authHeader(token)
	});
}
