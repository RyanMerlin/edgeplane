/**
 * ToolCallCard — renders a 'tool_call' ConversationItem.
 *
 * Shows: tool title, a status badge (using the Badge component from ui/),
 * and the rawInput in a collapsible JSON block.
 */

import { Badge } from '@/components/ui/badge';
import type { ConversationItem } from '@/lib/conversation/types';

type ToolCallItem = Extract<ConversationItem, { kind: 'tool_call' }>;

type BadgeVariant = 'default' | 'ok' | 'warn' | 'err' | 'accent' | 'purple';

function statusVariant(status: ToolCallItem['status']): BadgeVariant {
  switch (status) {
    case 'completed':
      return 'ok';
    case 'in_progress':
      return 'accent';
    case 'failed':
      return 'err';
    case 'cancelled':
      return 'default';
    case 'pending':
      return 'warn';
    default:
      return 'default';
  }
}

interface Props {
  item: ToolCallItem;
}

export function ToolCallCard({ item }: Props) {
  const hasInput = item.rawInput !== undefined && item.rawInput !== null;

  return (
    <div
      data-testid={`tool-call-card-${item.id}`}
      style={{
        padding: '5px 8px',
        background: 'var(--accent-bg)',
        border: '1px solid var(--accent-border)',
        fontSize: '12px',
        borderBottom: '1px solid var(--border)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
        <span
          style={{ color: 'var(--accent)', fontWeight: 600, fontFamily: 'monospace' }}
          data-testid={`tool-call-title-${item.id}`}
        >
          {item.title ?? item.toolCallId}
        </span>
        <Badge variant={statusVariant(item.status)} data-testid={`tool-call-status-${item.id}`}>
          {item.status}
        </Badge>
      </div>
      {hasInput && (
        <details style={{ marginTop: '4px' }}>
          <summary
            style={{
              cursor: 'pointer',
              fontSize: '11px',
              color: 'var(--muted)',
              userSelect: 'none',
              fontFamily: 'inherit',
            }}
          >
            input
          </summary>
          <pre
            style={{
              margin: '4px 0 0',
              padding: '4px',
              background: 'var(--base)',
              border: '1px solid var(--border)',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
              fontSize: '11px',
              lineHeight: '1.4',
            }}
          >
            {JSON.stringify(item.rawInput, null, 2)}
          </pre>
        </details>
      )}
    </div>
  );
}
