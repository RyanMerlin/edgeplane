/**
 * ThinkingBlock — renders a 'thinking' ConversationItem.
 *
 * Collapsed by default (the thinking monologue is noisy). Streaming-aware:
 * while streaming is true, a caret is shown in the summary line.
 */

import type { ConversationItem } from '@/lib/conversation/types';

type ThinkingItem = Extract<ConversationItem, { kind: 'thinking' }>;

interface Props {
  item: ThinkingItem;
}

export function ThinkingBlock({ item }: Props) {
  return (
    <details
      data-testid={`thinking-block-${item.id}`}
      style={{
        fontSize: '12px',
        color: 'var(--muted)',
        padding: '4px 0',
        borderBottom: '1px solid var(--border)',
      }}
    >
      <summary
        style={{ cursor: 'pointer', userSelect: 'none', padding: '2px 4px', fontFamily: 'inherit' }}
      >
        thinking
        {item.streaming && (
          <span
            aria-hidden="true"
            style={{
              display: 'inline-block',
              width: '2px',
              height: '0.85em',
              background: 'currentColor',
              marginLeft: '4px',
              verticalAlign: 'text-bottom',
              animation: 'acp-caret-blink 1s step-end infinite',
            }}
          />
        )}
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
        {item.text}
      </pre>
    </details>
  );
}
