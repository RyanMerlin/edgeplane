/**
 * ConversationView — transport-agnostic shell.
 *
 * Takes items + status + send/cancel handlers and renders:
 *   - Connection status indicator
 *   - Item list (auto-scroll to bottom on new items, with a "user scrolled up" guard)
 *   - Composer
 *
 * Both the agent view (ACP transport) and the future AI console (REST transport)
 * mount this component directly — neither leaks transport types here.
 */

import type { ConversationItem, TransportStatus } from '@/lib/conversation/types';
import { useEffect, useRef, useState } from 'react';
import { ApprovalPrompt } from './ApprovalPrompt';
import { Composer } from './Composer';
import { MessageBubble } from './MessageBubble';
import { ThinkingBlock } from './ThinkingBlock';
import { ToolCallCard } from './ToolCallCard';

// ── Status indicator ──────────────────────────────────────────────────────────

type StatusColor = 'var(--ok)' | 'var(--warn)' | 'var(--err)' | 'var(--muted)';

function statusColor(status: TransportStatus): StatusColor {
  switch (status) {
    case 'open':
      return 'var(--ok)';
    case 'connecting':
    case 'reconnecting':
      return 'var(--warn)';
    case 'error':
      return 'var(--err)';
    case 'closed':
      return 'var(--muted)';
  }
}

function StatusDot({ status }: { status: TransportStatus }) {
  return (
    <span
      aria-label={`connection status: ${status}`}
      title={status}
      style={{
        display: 'inline-block',
        width: '8px',
        height: '8px',
        borderRadius: '50%',
        background: statusColor(status),
        flexShrink: 0,
      }}
    />
  );
}

// ── Item renderer ─────────────────────────────────────────────────────────────

interface ItemRendererProps {
  item: ConversationItem;
  onApprove?: (id: string) => void;
  onReject?: (id: string, note: string) => void;
  approvalBusy?: boolean;
}

function ItemRenderer({ item, onApprove, onReject, approvalBusy }: ItemRendererProps) {
  switch (item.kind) {
    case 'message':
      return <MessageBubble item={item} />;
    case 'thinking':
      return <ThinkingBlock item={item} />;
    case 'tool_call':
      return <ToolCallCard item={item} />;
    case 'approval':
      return (
        <ApprovalPrompt item={item} onApprove={onApprove} onReject={onReject} busy={approvalBusy} />
      );
  }
}

// ── Main component ────────────────────────────────────────────────────────────

interface Props {
  items: ConversationItem[];
  status: TransportStatus;
  onSend: (text: string) => void;
  onCancel: () => void;
  /** Optional approval handlers — only needed when items may include 'approval' kind. */
  onApprove?: (actionId: string) => void;
  onReject?: (actionId: string, note: string) => void;
  /** Disable approval buttons while a mutation is in flight. */
  approvalBusy?: boolean;
}

// Scroll threshold: if the user has scrolled more than this many px from
// the bottom, we stop auto-scrolling.
const AUTOSCROLL_THRESHOLD = 80;

export function ConversationView({
  items,
  status,
  onSend,
  onCancel,
  onApprove,
  onReject,
  approvalBusy,
}: Props) {
  const listRef = useRef<HTMLDivElement>(null);
  const [userScrolledUp, setUserScrolledUp] = useState(false);
  const prevItemCountRef = useRef(items.length);

  // Track user scroll position
  function handleScroll() {
    const el = listRef.current;
    if (!el) return;
    const distFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setUserScrolledUp(distFromBottom > AUTOSCROLL_THRESHOLD);
  }

  // Auto-scroll to bottom when new items arrive (unless user scrolled up)
  useEffect(() => {
    const newCount = items.length;
    if (newCount <= prevItemCountRef.current) {
      prevItemCountRef.current = newCount;
      return;
    }
    prevItemCountRef.current = newCount;

    if (!userScrolledUp) {
      requestAnimationFrame(() => {
        const el = listRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });
    }
  }, [items, userScrolledUp]);

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        minHeight: 0,
        overflow: 'hidden',
        background: 'var(--base)',
        border: '1px solid var(--border)',
      }}
      data-testid="conversation-view"
    >
      {/* Status bar */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '6px',
          padding: '0 10px',
          height: '28px',
          flexShrink: 0,
          background: 'var(--surface)',
          borderBottom: '1px solid var(--border)',
          fontSize: '11px',
          color: 'var(--muted)',
        }}
        data-testid="connection-status"
      >
        <StatusDot status={status} />
        <span style={{ textTransform: 'uppercase', letterSpacing: '0.05em', fontWeight: 600 }}>
          {status}
        </span>
      </div>

      {/* Item list */}
      <div
        ref={listRef}
        onScroll={handleScroll}
        style={{
          flex: 1,
          minHeight: 0,
          overflowY: 'auto',
          padding: '8px',
          display: 'flex',
          flexDirection: 'column',
          gap: '2px',
        }}
        data-testid="conversation-list"
      >
        {items.length === 0 && (
          <div
            style={{ color: 'var(--muted)', fontSize: '13px', padding: '8px 0' }}
            data-testid="conversation-empty"
          >
            Waiting for the agent…
          </div>
        )}
        {items.map((item) => (
          <ItemRenderer
            key={item.id}
            item={item}
            onApprove={onApprove}
            onReject={onReject}
            approvalBusy={approvalBusy}
          />
        ))}
      </div>

      {/* Composer */}
      <Composer status={status} onSend={onSend} onCancel={onCancel} />
    </div>
  );
}
