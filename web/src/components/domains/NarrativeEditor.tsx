import { Suspense, lazy, useState } from 'react';

const MonacoEditor = lazy(() => import('@monaco-editor/react'));

interface NarrativeEditorProps {
  initialValue: string;
  onSave: (value: string) => void;
  isSaving?: boolean;
  saveError?: string | null;
  version?: number;
  modifiedAt?: string | null;
}

export function NarrativeEditor({
  initialValue,
  onSave,
  isSaving,
  saveError,
  version,
  modifiedAt,
}: NarrativeEditorProps) {
  const [value, setValue] = useState(initialValue);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', gap: 4 }}>
      <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
        <button
          type="button"
          data-testid="narrative-save-btn"
          disabled={isSaving}
          onClick={() => onSave(value)}
          style={{
            padding: '3px 10px',
            background: 'var(--accent-dim)',
            color: 'var(--accent)',
            border: '1px solid var(--accent)',
            borderRadius: 4,
            fontSize: 12,
            cursor: isSaving ? 'not-allowed' : 'pointer',
            fontFamily: 'var(--font)',
          }}
        >
          {isSaving ? 'Saving…' : 'Save'}
        </button>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <Suspense
          fallback={
            <div data-testid="editor-loading" style={{ color: 'var(--dim)', fontSize: 13 }}>
              Loading editor…
            </div>
          }
        >
          <MonacoEditor
            height="100%"
            language="markdown"
            theme="vs-dark"
            value={value}
            onChange={(v) => setValue(v ?? '')}
            options={{ minimap: { enabled: false }, wordWrap: 'on', fontSize: 13 }}
          />
        </Suspense>
      </div>
      {saveError && (
        <div
          data-testid="save-error"
          style={{ color: 'var(--err)', fontSize: 12, padding: '2px 0' }}
        >
          {saveError}
        </div>
      )}
      {(version !== undefined || modifiedAt) && (
        <div
          data-testid="editor-footer"
          style={{ fontSize: 11, color: 'var(--dim)', padding: '2px 0' }}
        >
          {version !== undefined && `v${version}`}
          {modifiedAt && ` · saved ${new Date(modifiedAt).toLocaleString()}`}
        </div>
      )}
    </div>
  );
}
