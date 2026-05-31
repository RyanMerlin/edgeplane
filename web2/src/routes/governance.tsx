import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/governance')({
  component: () => (
    <div className="pane-body" style={{ padding: '16px' }}>
      <div className="section-label">Governance</div>
      <p className="muted" style={{ fontSize: '12px' }}>
        Governance screen — deferred to after Phase 0.7 (utoipa/codegen).
      </p>
    </div>
  ),
});
