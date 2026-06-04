/**
 * Composer — textarea + Send + Cancel.
 *
 * Enter sends (matching the cmd/ctrl+Enter behavior from AgentConversation.svelte
 * is reversed here: plain Enter sends, Shift+Enter inserts a newline — standard
 * for chat UIs. This matches the spec: "Enter sends, Shift+Enter newlines").
 *
 * Disabled when status !== 'open'.
 * Cancel is visible while a turn is in flight (status === 'open' && the user has
 * sent a prompt). The caller controls the cancel action — Composer just surfaces
 * the button.
 */

import { Button } from '@/components/ui/button';
import type { TransportStatus } from '@/lib/conversation/types';
import { useState } from 'react';

interface Props {
  status: TransportStatus;
  onSend: (text: string) => void;
  onCancel: () => void;
}

export function Composer({ status, onSend, onCancel }: Props) {
  const [text, setText] = useState('');
  const isOpen = status === 'open';

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!text.trim() || !isOpen) return;
    onSend(text);
    setText('');
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      if (text.trim() && isOpen) {
        onSend(text);
        setText('');
      }
    }
    // Shift+Enter: default textarea behavior inserts a newline
  }

  return (
    <form
      onSubmit={handleSubmit}
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: '4px',
        padding: '6px 8px 8px',
        background: 'var(--surface)',
        borderTop: '1px solid var(--border)',
        flexShrink: 0,
      }}
    >
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Type a prompt — Enter to send, Shift+Enter for newline"
        rows={3}
        disabled={!isOpen}
        data-testid="composer-textarea"
        aria-label="Conversation input"
        style={{
          resize: 'vertical',
          fontFamily: 'inherit',
          fontSize: '13px',
          padding: '5px 7px',
          background: 'var(--base)',
          color: 'var(--text)',
          border: '1px solid var(--border-2)',
          width: '100%',
          boxSizing: 'border-box',
          opacity: isOpen ? 1 : 0.5,
        }}
      />
      <div style={{ display: 'flex', gap: '5px', justifyContent: 'flex-end' }}>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={onCancel}
          disabled={!isOpen}
          data-testid="composer-cancel"
        >
          Cancel turn
        </Button>
        <Button
          type="submit"
          variant="primary"
          size="sm"
          disabled={!isOpen || !text.trim()}
          data-testid="composer-send"
        >
          Send
        </Button>
      </div>
    </form>
  );
}
