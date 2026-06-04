/**
 * Normalized conversation item model.
 *
 * These types are the transport-agnostic contract between the ACP transport
 * (or the REST transport added in the next pass) and the shared conversation
 * components. Neither transport leaks wire-level types into the component tree.
 */

export type ConversationItem =
  | {
      kind: 'message';
      id: string;
      role: 'user' | 'assistant';
      text: string;
      streaming?: boolean;
    }
  | {
      kind: 'thinking';
      id: string;
      text: string;
      streaming?: boolean;
    }
  | {
      kind: 'tool_call';
      id: string;
      toolCallId: string;
      title?: string;
      status: 'pending' | 'in_progress' | 'completed' | 'failed' | 'cancelled';
      rawInput?: unknown;
    }
  | {
      // REST-only — the ACP transport never emits this variant.
      // Included now so ConversationView and future REST transport can share components.
      kind: 'approval';
      id: string;
      tool: string;
      args: unknown;
      reason: string;
      status: 'pending' | 'executed' | 'rejected';
    };

export type TransportStatus = 'connecting' | 'open' | 'reconnecting' | 'closed' | 'error';
