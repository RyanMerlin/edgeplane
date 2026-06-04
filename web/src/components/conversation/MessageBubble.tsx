/**
 * MessageBubble — renders a 'message' ConversationItem.
 *
 * User messages are right-aligned, assistant messages left-aligned.
 * The `streaming` flag adds a blinking caret affordance to signal
 * that text is still arriving.
 */

import type { ConversationItem } from '@/lib/conversation/types';

type MessageItem = Extract<ConversationItem, { kind: 'message' }>;

interface Props {
  item: MessageItem;
}

export function MessageBubble({ item }: Props) {
  const isUser = item.role === 'user';

  return (
    <div
      data-testid={`message-bubble-${item.id}`}
      style={{
        display: 'flex',
        justifyContent: isUser ? 'flex-end' : 'flex-start',
        padding: '2px 0',
      }}
    >
      <div
        style={{
          maxWidth: '80%',
          padding: '6px 10px',
          background: isUser ? 'var(--accent-bg)' : 'var(--surface)',
          border: `1px solid ${isUser ? 'var(--accent-border)' : 'var(--border)'}`,
          color: isUser ? 'var(--accent)' : 'var(--text)',
          fontSize: '13px',
          lineHeight: '1.45',
          whiteSpace: 'pre-wrap',
          wordBreak: 'break-word',
        }}
      >
        {item.text}
        {item.streaming && (
          <span
            aria-hidden="true"
            style={{
              display: 'inline-block',
              width: '2px',
              height: '1em',
              background: 'currentColor',
              marginLeft: '2px',
              verticalAlign: 'text-bottom',
              animation: 'acp-caret-blink 1s step-end infinite',
            }}
          />
        )}
      </div>
    </div>
  );
}
