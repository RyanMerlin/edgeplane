/**
 * ApprovalPrompt — renders an 'approval' ConversationItem with Approve / Reject buttons.
 *
 * Fleshed out for Phase 5 (REST transport). The approval item kind is only produced
 * by the REST transport — the ACP transport never emits it.
 *
 * Handlers are injected by the parent (the AI console) rather than discovered via
 * context, keeping the component transport-agnostic and easily testable.
 */

import type { ConversationItem } from '@/lib/conversation/types';
import { useState } from 'react';

type ApprovalItem = Extract<ConversationItem, { kind: 'approval' }>;

interface Props {
  item: ApprovalItem;
  /** Called when the user clicks Approve. */
  onApprove?: (id: string) => void;
  /** Called when the user clicks Reject. actionId and optional note are passed. */
  onReject?: (id: string, note: string) => void;
  /** Disable buttons while a mutation is in flight. */
  busy?: boolean;
}

// Derive the raw action id from the item id.
// item.id is one of:
//   "action-{actionId}"   (from pending_actions)
//   "evt-approval-{eventId}" (from approval_required events — no direct actionId)
// For the former, strip the prefix. For the latter, pass the full id.
function rawActionId(itemId: string): string {
  if (itemId.startsWith('action-')) return itemId.slice('action-'.length);
  return itemId;
}

export function ApprovalPrompt({ item, onApprove, onReject, busy = false }: Props) {
  const [note, setNote] = useState('');
  const [showNote, setShowNote] = useState(false);

  const isResolved = item.status !== 'pending';
  const disabled = busy || isResolved;

  const handleApprove = () => {
    if (disabled) return;
    onApprove?.(rawActionId(item.id));
  };

  const handleReject = () => {
    if (!showNote) {
      setShowNote(true);
      return;
    }
    if (disabled) return;
    onReject?.(rawActionId(item.id), note);
    setNote('');
    setShowNote(false);
  };

  const statusColor =
    item.status === 'executed'
      ? 'var(--ok)'
      : item.status === 'rejected'
        ? 'var(--err)'
        : 'var(--warn)';

  return (
    <div
      data-testid={`approval-prompt-${item.id}`}
      style={{
        padding: '8px 10px',
        background: 'var(--warn-bg, rgba(255,200,0,0.06))',
        border: '1px solid var(--warn-border, rgba(255,200,0,0.25))',
        borderBottom: '1px solid var(--border)',
        fontSize: '12px',
      }}
    >
      {/* Header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '6px',
          marginBottom: '4px',
        }}
      >
        <span
          style={{
            fontWeight: 600,
            color: statusColor,
            fontSize: '11px',
          }}
        >
          {isResolved ? `Approval ${item.status}` : 'Approval required'}
        </span>
        <code
          style={{
            fontSize: '11px',
            color: 'var(--accent)',
            background: 'var(--surface)',
            padding: '1px 4px',
            borderRadius: '3px',
          }}
        >
          {item.tool}
        </code>
      </div>

      {/* Reason */}
      <div style={{ color: 'var(--muted)', fontSize: '11px', marginBottom: '4px' }}>
        {item.reason}
      </div>

      {/* Args detail */}
      {item.args !== undefined && item.args !== null && (
        <details style={{ marginBottom: '6px' }}>
          <summary
            style={{
              cursor: 'pointer',
              fontSize: '11px',
              color: 'var(--dim)',
              userSelect: 'none',
            }}
          >
            arguments
          </summary>
          <pre
            style={{
              fontSize: '11px',
              margin: '3px 0 0',
              color: 'var(--text)',
              overflowX: 'auto',
            }}
          >
            {JSON.stringify(item.args, null, 2)}
          </pre>
        </details>
      )}

      {/* Rejection note input */}
      {showNote && !isResolved && (
        <div style={{ marginBottom: '6px' }}>
          <input
            type="text"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            placeholder="Rejection note (optional)"
            data-testid={`approval-reject-note-${item.id}`}
            style={{
              width: '100%',
              fontSize: '11px',
              padding: '3px 6px',
              background: 'var(--surface)',
              color: 'var(--text)',
              border: '1px solid var(--border)',
              borderRadius: '3px',
              boxSizing: 'border-box',
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleReject();
              if (e.key === 'Escape') {
                setShowNote(false);
                setNote('');
              }
            }}
          />
        </div>
      )}

      {/* Action buttons */}
      {!isResolved && (
        <div style={{ display: 'flex', gap: '5px' }}>
          <button
            type="button"
            className="primary"
            style={{ flex: 1, fontSize: '11px', padding: '3px 8px' }}
            onClick={handleApprove}
            disabled={disabled}
            data-testid={`approval-approve-${item.id}`}
          >
            Approve
          </button>
          <button
            type="button"
            className="ghost"
            style={{ flex: 1, fontSize: '11px', padding: '3px 8px' }}
            onClick={handleReject}
            disabled={busy}
            data-testid={`approval-reject-${item.id}`}
          >
            {showNote ? 'Confirm reject' : 'Reject'}
          </button>
          {showNote && (
            <button
              type="button"
              className="ghost"
              style={{ fontSize: '11px', padding: '3px 8px' }}
              onClick={() => {
                setShowNote(false);
                setNote('');
              }}
              data-testid={`approval-reject-cancel-${item.id}`}
            >
              Cancel
            </button>
          )}
        </div>
      )}

      {/* Resolved status */}
      {isResolved && (
        <div
          style={{
            fontSize: '11px',
            color: statusColor,
            marginTop: '4px',
            fontStyle: 'italic',
          }}
        >
          {item.status === 'executed' ? 'Approved and executed' : 'Rejected'}
        </div>
      )}
    </div>
  );
}
