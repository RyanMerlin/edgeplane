import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/ai')({
  component: () => (
    <div className="pane-body" style={{ padding: '16px' }}>
      <div className="section-label">Console</div>
      <p className="muted" style={{ fontSize: '12px' }}>
        AI console — coming in Phase 5.
      </p>
    </div>
  ),
});
