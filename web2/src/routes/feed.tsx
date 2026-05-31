import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/feed')({
  component: () => (
    <div className="pane-body" style={{ padding: '16px' }}>
      <div className="section-label">Feed</div>
      <p className="muted" style={{ fontSize: '12px' }}>
        Event feed — coming in Phase 3.
      </p>
    </div>
  ),
});
