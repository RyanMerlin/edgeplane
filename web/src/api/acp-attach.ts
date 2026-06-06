/**
 * Typed client for the ACP attach WebSocket.
 *
 * Ported from web/src/lib/api/acp-attach.ts (Svelte).
 *
 * Speaks the same wire protocol as edgeplaned's `pump_acp`:
 *
 *   Outbound (agent → viewer):
 *     - One-shot `{kind:"hello", protocol:"acp/1"}` on connect.
 *     - `SessionNotification` JSON per `session/update`.
 *
 *   Inbound (viewer → agent):
 *     - `{kind:"prompt", text:string}` → `AgentSignal::UserInput`
 *     - `{kind:"cancel"}`              → `AgentSignal::Cancel`
 *
 * NO xterm.js, NO binary frames.
 */

// ── URL helper ─────────────────────────────────────────────────────────────────

export function attachAgentWsUrl(nodeId: string, agentId: string): string {
  if (typeof window === 'undefined') {
    throw new Error('attachAgentWsUrl can only run in the browser');
  }
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const base = `${proto}//${window.location.host}`;
  const path = `/api/runtime/nodes/${encodeURIComponent(nodeId)}/agents/${encodeURIComponent(agentId)}/attach`;
  return `${base}${path}`;
}

// ── Wire types (loose mirrors of the Rust shapes) ──────────────────────────────

/** Hello frame the edgeplaned pump sends on connect. */
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

/**
 * Discriminated union over the SessionUpdate variants we recognise.
 * Variants we don't model fall through to `UnknownUpdate` so a
 * forward-compatible agent can ship new kinds without breaking the renderer.
 */
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

// ── Connection wrapper ─────────────────────────────────────────────────────────

export type AcpConnectionStatus = 'connecting' | 'open' | 'reconnecting' | 'closed' | 'error';

export interface AcpAttachHandlers {
  /** Connection state transitions. */
  onStatus?(status: AcpConnectionStatus, detail?: string): void;
  /** First frame after WS open. Always (today) the hello envelope. */
  onHello?(frame: HelloFrame): void;
  /** Every parseable `SessionNotification`. */
  onNotification?(notif: SessionNotification): void;
  /** Frames we couldn't parse — useful for diagnostics. Default: drop. */
  onUnparseable?(text: string, error: unknown): void;
}

/**
 * Live attach connection. Reconnects with exponential backoff (1s → 30s)
 * on close/error so a brief network blip doesn't drop the viewer.
 *
 * open() is idempotent — repeated calls before dispose() are no-ops.
 * This guards against React 19 StrictMode double-mount (see useAcpConversation).
 */
export class AcpAttach {
  private ws: WebSocket | undefined;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private reconnectDelayMs = 1000;
  private disposed = false;
  // Track whether an open() is already pending/active so double-mount is safe.
  private _opened = false;
  private readonly nodeId: string;
  private readonly agentId: string;
  private readonly handlers: AcpAttachHandlers;

  constructor(nodeId: string, agentId: string, handlers: AcpAttachHandlers) {
    this.nodeId = nodeId;
    this.agentId = agentId;
    this.handlers = handlers;
  }

  /** Begin connecting. Idempotent — repeated calls before dispose() are no-ops. */
  open(): void {
    if (this.disposed || this._opened) return;
    this._opened = true;
    this._doOpen();
  }

  private _doOpen(): void {
    if (this.disposed) return;
    this.handlers.onStatus?.('connecting');
    let url: string;
    try {
      url = attachAgentWsUrl(this.nodeId, this.agentId);
    } catch (err) {
      this.handlers.onStatus?.('error', err instanceof Error ? err.message : String(err));
      this.scheduleReconnect();
      return;
    }
    try {
      this.ws = new WebSocket(url);
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

  /**
   * Send a user prompt. Drops silently if the socket isn't open.
   * Caller is expected to gate prompts on status === 'open'.
   */
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
    if (typeof data !== 'string') return; // ACP doesn't use binary frames
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
    this.handlers.onStatus?.('reconnecting');
    this.reconnectTimer = setTimeout(() => {
      this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 1.5, 30000);
      this._doOpen();
    }, this.reconnectDelayMs);
  }
}
