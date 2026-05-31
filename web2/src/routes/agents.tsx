import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/agents')({
  component: () => (
    <div className="pane-body" style={{ padding: '16px' }}>
      <div className="section-label">Agents</div>
      <p className="muted" style={{ fontSize: '12px' }}>
        Agent list — coming in Phase 2.
      </p>
    </div>
  ),
});
