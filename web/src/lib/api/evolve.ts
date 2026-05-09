import { request, authHeader } from './client';

export type EvolveRun = {
	run_id: string;
	agent: string;
	started_at: string;
	status: string;
	ai_session_id?: string | null;
};

export type EvolveMissionStatus = {
	mission_id: string;
	status: string;
	created_at: string;
	task_count: number;
	run_count: number;
	runs: EvolveRun[];
};

export function seedEvolveMission(spec: Record<string, unknown>, token?: string) {
	return request<unknown>('/evolve/missions', {
		method: 'POST',
		headers: authHeader(token),
		body: JSON.stringify({ spec })
	});
}

export function runEvolveMission(
	missionId: string,
	runtimeKind = 'opencode',
	policy: Record<string, unknown> = {},
	token?: string
) {
	return request<unknown>(`/evolve/missions/${encodeURIComponent(missionId)}/run`, {
		method: 'POST',
		headers: authHeader(token),
		body: JSON.stringify({ runtime_kind: runtimeKind, policy })
	});
}

export function getEvolveMissionStatus(missionId: string, token?: string) {
	return request<EvolveMissionStatus>(
		`/evolve/missions/${encodeURIComponent(missionId)}/status`,
		{ headers: authHeader(token) }
	);
}
