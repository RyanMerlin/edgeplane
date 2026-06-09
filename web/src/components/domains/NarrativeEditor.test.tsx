import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@monaco-editor/react', () => ({
  default: ({ value, onChange }: { value: string; onChange?: (v: string | undefined) => void }) => (
    <textarea
      data-testid="monaco-editor"
      defaultValue={value}
      onChange={(e) => onChange?.(e.target.value)}
    />
  ),
}));

import { NarrativeEditor } from './NarrativeEditor';

describe('NarrativeEditor', () => {
  it('renders Save button', () => {
    render(<NarrativeEditor initialValue="# Hello" onSave={vi.fn()} />);
    expect(screen.getByTestId('narrative-save-btn')).toBeInTheDocument();
  });

  it('calls onSave with current value when Save clicked', () => {
    const onSave = vi.fn();
    render(<NarrativeEditor initialValue="# Hello" onSave={onSave} />);
    fireEvent.click(screen.getByTestId('narrative-save-btn'));
    expect(onSave).toHaveBeenCalledWith('# Hello');
  });

  it('shows saving state while isSaving is true', () => {
    render(<NarrativeEditor initialValue="" onSave={vi.fn()} isSaving />);
    expect(screen.getByTestId('narrative-save-btn')).toBeDisabled();
    expect(screen.getByText('Saving…')).toBeInTheDocument();
  });

  it('shows save error when saveError is set', () => {
    render(<NarrativeEditor initialValue="" onSave={vi.fn()} saveError="Network error" />);
    expect(screen.getByTestId('save-error')).toHaveTextContent('Network error');
  });

  it('shows version footer when version and modifiedAt provided', () => {
    render(
      <NarrativeEditor
        initialValue=""
        onSave={vi.fn()}
        version={3}
        modifiedAt="2026-06-01T10:00:00Z"
      />,
    );
    const footer = screen.getByTestId('editor-footer');
    expect(footer).toHaveTextContent('v3');
  });
});
