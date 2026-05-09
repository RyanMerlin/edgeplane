import { request, authHeader } from './client';

export type PolicySummary = {
	version?: string;
	name?: string;
	description?: string;
};

export function fetchPolicy(token?: string) {
	return request<PolicySummary>('/governance/policy/active', { headers: authHeader(token) });
}

export async function fetchGovernanceEvents(token?: string): Promise<unknown[]> {
	try {
		return await request<unknown[]>('/governance/policy/events?limit=10', {
			headers: authHeader(token)
		});
	} catch {
		return [];
	}
}
