/**
 * Typed client for the ACP attach WebSocket.
 *
 * Speaks the same wire protocol as mcd's `pump_acp`
 * (integrations/mcd/crates/mcd/src/attach_ws.rs):
 *
 *   Outbound (agent → viewer):
 *     - One-shot `{kind:"hello", protocol:"acp/1"}` on connect.
 *     - `SessionNotification` JSON per `session/update`.
 *
 *   Inbound (viewer → agent):
 *     - `{kind:"prompt", text:string}` → `AgentSignal::UserInput`
 *     - `{kind:"cancel"}`              → `AgentSignal::Cancel`
 *
 * NO xterm.js, NO binary frames — see C.4 of
 * docs/plans/2026-05-11-retire-tmux-via-acp-persistent-sessions.md.
 */

import { attachAgentWsUrl } from './agents';

// ── Wire types (loose mirrors of the Rust shapes) ──────────────────────────

/** Hello frame the mcd pump sends on connect. */
export interface HelloFrame {
	kind: 'hello';
	protocol: string;
}

/** A `session/update` notification from the agent. */
export interface SessionNotification {
	sessionId: string;
	update: SessionUpdate;
	_meta?: Record<string, unknown>;
}

/** Discriminated union over the SessionUpdate variants we recognise.
 * Variants we don't model fall through to `Unknown` so a forward-compatible
 * agent can ship new kinds without breaking the renderer. */
export type SessionUpdate =
	| AgentMessageChunk
	| AgentThoughtChunk
	| ToolCallUpdate
	| ToolCallUpdateChange
	| PlanUpdate
	| UnknownUpdate;

export interface ContentBlock {
	type?: 'text' | string;
	text?: string;
	[k: string]: unknown;
}

export interface AgentMessageChunk {
	sessionUpdate: 'agent_message_chunk';
	content: ContentBlock;
}

export interface AgentThoughtChunk {
	sessionUpdate: 'agent_thought_chunk';
	content: ContentBlock;
}

export interface ToolCallUpdate {
	sessionUpdate: 'tool_call';
	toolCallId?: string;
	title?: string;
	kind?: string;
	rawInput?: unknown;
	[k: string]: unknown;
}

export interface ToolCallUpdateChange {
	sessionUpdate: 'tool_call_update';
	toolCallId?: string;
	title?: string;
	status?: 'pending' | 'in_progress' | 'completed' | 'failed' | 'cancelled' | string;
	[k: string]: unknown;
}

export interface PlanUpdate {
	sessionUpdate: 'plan';
	[k: string]: unknown;
}

export interface UnknownUpdate {
	sessionUpdate: string;
	[k: string]: unknown;
}

// ── Connection wrapper ─────────────────────────────────────────────────────

export type ConnectionStatus = 'connecting' | 'open' | 'closed' | 'error';

export interface AcpAttachHandlers {
	/** Connection state transitions. */
	onStatus?(status: ConnectionStatus, detail?: string): void;
	/** First frame after WS open. Always (today) the hello envelope. */
	onHello?(frame: HelloFrame): void;
	/** Every parseable `SessionNotification`. */
	onNotification?(notif: SessionNotification): void;
	/** Frames we couldn't parse — useful for diagnostics. Default: drop. */
	onUnparseable?(text: string, error: unknown): void;
}

/** Live attach connection. Reconnects with exponential backoff (1s → 30s)
 * on close/error so a brief network blip doesn't drop the viewer. */
export class AcpAttach {
	private ws: WebSocket | undefined;
	private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
	private reconnectDelayMs = 1000;
	private disposed = false;

	constructor(
		private readonly nodeId: string,
		private readonly agentId: string,
		private readonly token: string,
		private readonly handlers: AcpAttachHandlers
	) {}

	/** Begin connecting. Idempotent — repeated calls before `dispose()`
	 * are no-ops. */
	open(): void {
		if (this.disposed) return;
		this.handlers.onStatus?.('connecting');
		try {
			this.ws = new WebSocket(attachAgentWsUrl(this.nodeId, this.agentId, this.token));
		} catch (err) {
			this.handlers.onStatus?.('error', err instanceof Error ? err.message : String(err));
			this.scheduleReconnect();
			return;
		}

		this.ws.onopen = () => {
			this.reconnectDelayMs = 1000;
			this.handlers.onStatus?.('open');
		};
		this.ws.onmessage = (ev) => this.handleFrame(ev.data);
		this.ws.onerror = () => {
			this.handlers.onStatus?.('error', 'websocket error');
		};
		this.ws.onclose = () => {
			this.handlers.onStatus?.('closed');
			this.scheduleReconnect();
		};
	}

	/** Send a user prompt. Buffers nothing — drops silently if the socket
	 * isn't open. Caller is expected to gate prompts on status === 'open'. */
	sendPrompt(text: string): boolean {
		if (!text.trim() || this.ws?.readyState !== WebSocket.OPEN) return false;
		this.ws.send(JSON.stringify({ kind: 'prompt', text }));
		return true;
	}

	/** Cancel the active turn. */
	sendCancel(): boolean {
		if (this.ws?.readyState !== WebSocket.OPEN) return false;
		this.ws.send(JSON.stringify({ kind: 'cancel' }));
		return true;
	}

	dispose(): void {
		this.disposed = true;
		clearTimeout(this.reconnectTimer);
		try {
			this.ws?.close();
		} catch {
			// ignore
		}
	}

	private handleFrame(data: unknown): void {
		if (typeof data !== 'string') return; // ACP doesn't use binary
		let parsed: unknown;
		try {
			parsed = JSON.parse(data);
		} catch (err) {
			this.handlers.onUnparseable?.(data, err);
			return;
		}
		const obj = parsed as Record<string, unknown>;
		if (obj && obj.kind === 'hello') {
			this.handlers.onHello?.(obj as unknown as HelloFrame);
			return;
		}
		if (obj && typeof obj.sessionId === 'string' && obj.update) {
			this.handlers.onNotification?.(obj as unknown as SessionNotification);
			return;
		}
		this.handlers.onUnparseable?.(data, new Error('unrecognised frame shape'));
	}

	private scheduleReconnect(): void {
		if (this.disposed) return;
		clearTimeout(this.reconnectTimer);
		this.reconnectTimer = setTimeout(() => {
			this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 1.5, 30000);
			this.open();
		}, this.reconnectDelayMs);
	}
}
