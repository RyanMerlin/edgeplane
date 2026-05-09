import { request, authHeader } from './client';

export type ScheduledJob = {
	id: number;
	owner_subject: string;
	name: string;
	description: string;
	cron_expr: string;
	runtime_kind: string;
	initial_prompt: string;
	system_context?: string | null;
	policy: Record<string, unknown>;
	enabled: boolean;
	last_run_at?: string | null;
	last_session_id?: string | null;
	created_at: string;
	updated_at: string;
};

export type CreateJobData = {
	name: string;
	cron_expr: string;
	initial_prompt: string;
	description?: string;
	runtime_kind?: string;
	system_context?: string;
	policy?: Record<string, unknown>;
	enabled?: boolean;
};

export type UpdateJobData = Partial<{
	name: string;
	description: string;
	cron_expr: string;
	runtime_kind: string;
	initial_prompt: string;
	system_context: string;
	policy: Record<string, unknown>;
	enabled: boolean;
}>;

export function listScheduledJobs(token?: string) {
	return request<ScheduledJob[]>('/scheduled-jobs', { headers: authHeader(token) });
}

export function createScheduledJob(data: CreateJobData, token?: string) {
	return request<ScheduledJob>('/scheduled-jobs', {
		method: 'POST',
		headers: authHeader(token),
		body: JSON.stringify(data)
	});
}

export function getScheduledJob(jobId: number, token?: string) {
	return request<ScheduledJob>(`/scheduled-jobs/${jobId}`, { headers: authHeader(token) });
}

export function updateScheduledJob(jobId: number, data: UpdateJobData, token?: string) {
	return request<ScheduledJob>(`/scheduled-jobs/${jobId}`, {
		method: 'PUT',
		headers: authHeader(token),
		body: JSON.stringify(data)
	});
}

export function deleteScheduledJob(jobId: number, token?: string) {
	return request<unknown>(`/scheduled-jobs/${jobId}`, {
		method: 'DELETE',
		headers: authHeader(token)
	});
}

export function triggerScheduledJobNow(jobId: number, token?: string) {
	return request<unknown>(`/scheduled-jobs/${jobId}/run`, {
		method: 'POST',
		headers: authHeader(token)
	});
}
