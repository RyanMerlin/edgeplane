/**
 * ApprovalPrompt — stub for the 'approval' ConversationItem kind.
 *
 * The approval item kind is produced only by the REST transport (next pass).
 * This stub renders a minimal affordance so ConversationView can handle the
 * type union today without panicking. The REST transport pass will flesh this out.
 */

import type { ConversationItem } from '@/lib/conversation/types';

type ApprovalItem = Extract<ConversationItem, { kind: 'approval' }>;

interface Props {
  item: ApprovalItem;
}

export function ApprovalPrompt({ item }: Props) {
  return (
    <div
      data-testid={`approval-prompt-${item.id}`}
      style={{
        padding: '8px 10px',
        background: 'var(--warn-bg)',
        border: '1px solid var(--warn-border)',
        fontSize: '12px',
        borderBottom: '1px solid var(--border)',
      }}
    >
      <div style={{ color: 'var(--warn)', fontWeight: 600, marginBottom: '4px' }}>
        Approval required: {item.tool}
      </div>
      <div style={{ color: 'var(--muted)', fontSize: '11px' }}>{item.reason}</div>
      <div style={{ marginTop: '6px', fontSize: '11px', color: 'var(--dim)' }}>
        Status: {item.status}
      </div>
    </div>
  );
}
