import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/explorer')({
  component: () => (
    <div className="pane-body" style={{ padding: '16px' }}>
      <div className="section-label">Explorer</div>
      <p className="muted" style={{ fontSize: '12px' }}>
        Explorer tree — coming in Phase 2.
      </p>
    </div>
  ),
});
