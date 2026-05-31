import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/')({
  component: IndexPage,
});

function IndexPage() {
  return (
    <div className="pane-body" style={{ padding: '16px' }}>
      <div className="section-label">Overview</div>
      <p className="muted" style={{ fontSize: '12px' }}>
        Fleet dashboard — coming in Phase 4.
      </p>
    </div>
  );
}
