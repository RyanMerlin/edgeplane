import { request, authHeader } from './client';

// ── Types ─────────────────────────────────────────────────────────────────────

export type RuntimeKind = 'opencode' | 'claude_code' | 'codex' | 'native';

export type RuntimePolicy = {
	allowed_tools: string[];
	denied_tools: string[];
	max_turns_per_session: number;
	require_approval_for_writes: boolean;
	workspace_ttl_seconds: number;
};

export type CapabilitySet = {
	runtime_kind: RuntimeKind;
	display_name: string;
	icon_slug: string;
	supports_streaming: boolean;
	supports_file_workspace: boolean;
	supports_tool_interception: boolean;
	supports_skill_packs: boolean;
	supports_session_resume: boolean;
	max_context_tokens: number;
};

export type NormalizedEventFamily = 'lifecycle' | 'io' | 'tool' | 'approval' | 'view' | 'runtime';

export type NormalizedEvent = {
	schema_version: 1;
	family: NormalizedEventFamily;
	event_type: string;
	session_id: string;
	turn_id: number | null;
	runtime_kind: string;
	payload: Record<string, unknown>;
	created_at: string;
};

export type AiTurn = {
	id: number;
	role: 'user' | 'assistant' | 'tool';
	content: Record<string, unknown>;
	created_at: string;
};

export type AiEvent = {
	id: number;
	turn_id?: number | null;
	event_type: string;
	payload: Record<string, unknown>;
	created_at: string;
};

export type AiPendingAction = {
	id: string;
	tool: string;
	args: Record<string, unknown>;
	reason: string;
	status: string;
	requested_by: string;
	approved_by: string;
	rejected_by: string;
	rejection_note: string;
	created_at: string;
	updated_at: string;
};

export type AiSession = {
	id: string;
	owner_subject: string;
	title: string;
	status: string;
	runtime_kind?: string;
	runtime_session_id?: string | null;
	workspace_path?: string | null;
	capability_snapshot?: Record<string, unknown>;
	policy?: Record<string, unknown>;
	turns: AiTurn[];
	events: AiEvent[];
	pending_actions: AiPendingAction[];
	created_at: string;
	updated_at: string;
};

// ── Functions ─────────────────────────────────────────────────────────────────

export function listRuntimeCapabilities(token?: string) {
	return request<CapabilitySet[]>('/ai/runtime-capabilities', { headers: authHeader(token) });
}

export function createAiSession(
	token?: string,
	title = '',
	runtimeKind?: string,
	policy?: Record<string, unknown>
) {
	return request<AiSession>('/ai/sessions', {
		method: 'POST',
		headers: authHeader(token),
		body: JSON.stringify({ title, runtime_kind: runtimeKind ?? 'opencode', policy: policy ?? {} })
	});
}

export function createAiSessionWithRuntime(
	token: string | undefined,
	opts: { title?: string; runtime_kind?: RuntimeKind; policy?: Partial<RuntimePolicy> }
) {
	return request<AiSession>('/ai/sessions', {
		method: 'POST',
		headers: authHeader(token),
		body: JSON.stringify({
			title: opts.title ?? '',
			runtime_kind: opts.runtime_kind ?? 'opencode',
			policy: opts.policy ?? {}
		})
	});
}

export function listAiSessions(token?: string) {
	return request<AiSession[]>('/ai/sessions?limit=20', { headers: authHeader(token) });
}

export function getAiSession(sessionId: string, token?: string, sinceEventId = 0) {
	return request<AiSession>(
		`/ai/sessions/${encodeURIComponent(sessionId)}?since_event_id=${sinceEventId}`,
		{ headers: authHeader(token) }
	);
}

export function sendAiTurn(sessionId: string, message: string, token?: string) {
	return request<AiSession>(`/ai/sessions/${encodeURIComponent(sessionId)}/turns`, {
		method: 'POST',
		headers: authHeader(token),
		body: JSON.stringify({ message })
	});
}

export function approveAiAction(sessionId: string, actionId: string, token?: string) {
	return request<AiSession>(
		`/ai/sessions/${encodeURIComponent(sessionId)}/actions/${encodeURIComponent(actionId)}/approve`,
		{ method: 'POST', headers: authHeader(token) }
	);
}

export function rejectAiAction(sessionId: string, actionId: string, token?: string, note = '') {
	return request<AiSession>(
		`/ai/sessions/${encodeURIComponent(sessionId)}/actions/${encodeURIComponent(actionId)}/reject?note=${encodeURIComponent(note)}`,
		{ method: 'POST', headers: authHeader(token) }
	);
}
