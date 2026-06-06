/**
 * Shared conversation component tests.
 *
 * Tests:
 *   MessageBubble:
 *     - renders assistant message text
 *     - renders user message text
 *     - shows streaming caret when streaming=true
 *
 *   ThinkingBlock:
 *     - renders thinking text in a details element
 *     - shows streaming caret when streaming=true
 *
 *   ToolCallCard:
 *     - renders tool title
 *     - renders status badge
 *     - shows rawInput block when present
 *     - no input block when rawInput is absent
 *
 *   Composer:
 *     - disabled when status !== 'open'
 *     - enabled when status === 'open'
 *     - calls onSend when Send button is clicked
 *     - calls onSend on Enter key
 *     - does NOT send on Shift+Enter
 *     - calls onCancel when Cancel button is clicked
 *     - Send button disabled when input is empty
 */

import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Composer } from './Composer';
import { MessageBubble } from './MessageBubble';
import { ThinkingBlock } from './ThinkingBlock';
import { ToolCallCard } from './ToolCallCard';

// ── MessageBubble ─────────────────────────────────────────────────────────────

describe('MessageBubble', () => {
  it('renders assistant message text', () => {
    render(
      <MessageBubble item={{ kind: 'message', id: 'm1', role: 'assistant', text: 'Hello!' }} />,
    );
    expect(screen.getByTestId('message-bubble-m1')).toHaveTextContent('Hello!');
  });

  it('renders user message text', () => {
    render(
      <MessageBubble item={{ kind: 'message', id: 'm2', role: 'user', text: 'My question' }} />,
    );
    expect(screen.getByTestId('message-bubble-m2')).toHaveTextContent('My question');
  });

  it('shows streaming caret when streaming=true', () => {
    const { container } = render(
      <MessageBubble
        item={{ kind: 'message', id: 'm3', role: 'assistant', text: 'hello', streaming: true }}
      />,
    );
    // Caret is an aria-hidden span with animation style
    const caret = container.querySelector('span[aria-hidden]');
    expect(caret).not.toBeNull();
  });

  it('does not show streaming caret when streaming is false', () => {
    const { container } = render(
      <MessageBubble
        item={{ kind: 'message', id: 'm4', role: 'assistant', text: 'done', streaming: false }}
      />,
    );
    const caret = container.querySelector('span[aria-hidden]');
    expect(caret).toBeNull();
  });
});

// ── ThinkingBlock ─────────────────────────────────────────────────────────────

describe('ThinkingBlock', () => {
  it('renders thinking text', () => {
    render(<ThinkingBlock item={{ kind: 'thinking', id: 't1', text: 'I am thinking...' }} />);
    expect(screen.getByTestId('thinking-block-t1')).toBeInTheDocument();
    expect(screen.getByText('I am thinking...')).toBeInTheDocument();
  });

  it('is a details element (collapsed by default)', () => {
    const { container } = render(
      <ThinkingBlock item={{ kind: 'thinking', id: 't2', text: 'stuff' }} />,
    );
    const details = container.querySelector('details');
    expect(details).not.toBeNull();
    expect(details?.open).toBe(false);
  });

  it('shows streaming caret when streaming=true', () => {
    const { container } = render(
      <ThinkingBlock item={{ kind: 'thinking', id: 't3', text: 'hmm', streaming: true }} />,
    );
    const caret = container.querySelector('span[aria-hidden]');
    expect(caret).not.toBeNull();
  });
});

// ── ToolCallCard ──────────────────────────────────────────────────────────────

describe('ToolCallCard', () => {
  it('renders tool title', () => {
    render(
      <ToolCallCard
        item={{
          kind: 'tool_call',
          id: 'tc1',
          toolCallId: 'tc-x',
          title: 'read_file',
          status: 'pending',
        }}
      />,
    );
    expect(screen.getByTestId('tool-call-title-tc1')).toHaveTextContent('read_file');
  });

  it('renders status badge', () => {
    render(
      <ToolCallCard
        item={{
          kind: 'tool_call',
          id: 'tc2',
          toolCallId: 'tc-y',
          title: 'write_file',
          status: 'completed',
        }}
      />,
    );
    expect(screen.getByTestId('tool-call-status-tc2')).toHaveTextContent('completed');
  });

  it('shows rawInput block when rawInput is present', () => {
    render(
      <ToolCallCard
        item={{
          kind: 'tool_call',
          id: 'tc3',
          toolCallId: 'tc-z',
          title: 'bash',
          status: 'in_progress',
          rawInput: { cmd: 'ls -la' },
        }}
      />,
    );
    // details element for input should be present
    const card = screen.getByTestId('tool-call-card-tc3');
    const details = card.querySelector('details');
    expect(details).not.toBeNull();
  });

  it('does not show rawInput block when rawInput is absent', () => {
    render(
      <ToolCallCard
        item={{
          kind: 'tool_call',
          id: 'tc4',
          toolCallId: 'tc-w',
          title: 'noop',
          status: 'pending',
        }}
      />,
    );
    const card = screen.getByTestId('tool-call-card-tc4');
    const details = card.querySelector('details');
    expect(details).toBeNull();
  });

  it('falls back to toolCallId when title is absent', () => {
    render(
      <ToolCallCard
        item={{
          kind: 'tool_call',
          id: 'tc5',
          toolCallId: 'my-tool-id',
          status: 'pending',
        }}
      />,
    );
    expect(screen.getByTestId('tool-call-title-tc5')).toHaveTextContent('my-tool-id');
  });
});

// ── Composer ──────────────────────────────────────────────────────────────────

describe('Composer', () => {
  it('is disabled when status is not open', () => {
    render(<Composer status="connecting" onSend={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByTestId('composer-textarea')).toBeDisabled();
    expect(screen.getByTestId('composer-send')).toBeDisabled();
    expect(screen.getByTestId('composer-cancel')).toBeDisabled();
  });

  it.each(['closed', 'reconnecting', 'error'] as const)(
    'is disabled when status is %s',
    (status) => {
      render(<Composer status={status} onSend={vi.fn()} onCancel={vi.fn()} />);
      expect(screen.getByTestId('composer-textarea')).toBeDisabled();
    },
  );

  it('is enabled when status is open', () => {
    render(<Composer status="open" onSend={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByTestId('composer-textarea')).not.toBeDisabled();
  });

  it('calls onSend with text when Send button is clicked', () => {
    const onSend = vi.fn();
    render(<Composer status="open" onSend={onSend} onCancel={vi.fn()} />);

    fireEvent.change(screen.getByTestId('composer-textarea'), {
      target: { value: 'hello world' },
    });
    fireEvent.click(screen.getByTestId('composer-send'));

    expect(onSend).toHaveBeenCalledWith('hello world');
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it('calls onSend on Enter key', () => {
    const onSend = vi.fn();
    render(<Composer status="open" onSend={onSend} onCancel={vi.fn()} />);

    const ta = screen.getByTestId('composer-textarea');
    fireEvent.change(ta, { target: { value: 'send this' } });
    fireEvent.keyDown(ta, { key: 'Enter', shiftKey: false });

    expect(onSend).toHaveBeenCalledWith('send this');
  });

  it('does NOT call onSend on Shift+Enter', () => {
    const onSend = vi.fn();
    render(<Composer status="open" onSend={onSend} onCancel={vi.fn()} />);

    const ta = screen.getByTestId('composer-textarea');
    fireEvent.change(ta, { target: { value: 'multi\nline' } });
    fireEvent.keyDown(ta, { key: 'Enter', shiftKey: true });

    expect(onSend).not.toHaveBeenCalled();
  });

  it('calls onCancel when Cancel button is clicked', () => {
    const onCancel = vi.fn();
    render(<Composer status="open" onSend={vi.fn()} onCancel={onCancel} />);
    fireEvent.click(screen.getByTestId('composer-cancel'));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('Send button is disabled when input is empty', () => {
    render(<Composer status="open" onSend={vi.fn()} onCancel={vi.fn()} />);
    // Input starts empty
    expect(screen.getByTestId('composer-send')).toBeDisabled();
  });

  it('Send button is disabled for whitespace-only input', () => {
    render(<Composer status="open" onSend={vi.fn()} onCancel={vi.fn()} />);
    fireEvent.change(screen.getByTestId('composer-textarea'), { target: { value: '   ' } });
    expect(screen.getByTestId('composer-send')).toBeDisabled();
  });

  it('clears input after Send', () => {
    render(<Composer status="open" onSend={vi.fn()} onCancel={vi.fn()} />);
    const ta = screen.getByTestId('composer-textarea') as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: 'some text' } });
    fireEvent.click(screen.getByTestId('composer-send'));
    expect(ta.value).toBe('');
  });
});
