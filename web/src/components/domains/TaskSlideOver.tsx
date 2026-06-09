interface TaskRecord {
  id: number;
  public_id: string;
  mission_id: string;
  title: string;
  description?: string | null;
  status: string;
  owner?: string | null;
  contributors?: string | null;
  created_at: string;
  updated_at: string;
}

interface TaskSlideOverProps {
  task: TaskRecord | null;
  isOpen: boolean;
  onClose: () => void;
}

export function TaskSlideOver({ task, isOpen, onClose }: TaskSlideOverProps) {
  if (!isOpen || !task) return null;

  return (
    <>
      <div
        data-testid="slide-over-backdrop"
        onClick={onClose}
        onKeyDown={(e) => e.key === 'Escape' && onClose()}
        role="button"
        tabIndex={-1}
        style={{
          position: 'fixed',
          inset: 0,
          background: 'rgba(0,0,0,0.4)',
          zIndex: 40,
        }}
      />
      <div
        data-testid="slide-over"
        style={{
          position: 'fixed',
          top: 0,
          right: 0,
          bottom: 0,
          width: 420,
          background: 'var(--frame)',
          borderLeft: '1px solid var(--border)',
          zIndex: 50,
          display: 'flex',
          flexDirection: 'column',
          padding: '16px 20px',
          overflowY: 'auto',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 16 }}>
          <span style={{ fontSize: 14, fontWeight: 590, color: 'var(--text)', flex: 1 }}>
            {task.title}
          </span>
          <button
            type="button"
            data-testid="slide-over-close"
            onClick={onClose}
            style={{
              background: 'none',
              border: 'none',
              color: 'var(--dim)',
              cursor: 'pointer',
              fontSize: 16,
            }}
          >
            ✕
          </button>
        </div>
        <dl
          style={{
            display: 'grid',
            gridTemplateColumns: 'max-content 1fr',
            gap: '6px 16px',
            fontSize: 13,
          }}
        >
          <dt style={{ color: 'var(--dim)' }}>Status</dt>
          <dd style={{ margin: 0, color: 'var(--accent)' }}>{task.status}</dd>
          <dt style={{ color: 'var(--dim)' }}>ID</dt>
          <dd
            style={{ margin: 0, color: 'var(--text-2)', fontFamily: 'var(--mono)', fontSize: 12 }}
          >
            {task.public_id}
          </dd>
          {task.owner && (
            <>
              <dt style={{ color: 'var(--dim)' }}>Owner</dt>
              <dd style={{ margin: 0, color: 'var(--text)' }}>{task.owner}</dd>
            </>
          )}
        </dl>
        {task.description && (
          <p style={{ marginTop: 16, fontSize: 13, color: 'var(--text-2)', lineHeight: 1.6 }}>
            {task.description}
          </p>
        )}
      </div>
    </>
  );
}
