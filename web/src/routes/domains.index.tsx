import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { Link, createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';

type ExplorerDomainNode = components['schemas']['ExplorerDomainNode'];
type ExplorerMissionNode = components['schemas']['ExplorerMissionNode'];

export const Route = createFileRoute('/domains/')({
  component: DomainsOverviewPage,
});

export function DomainsOverviewPage() {
  const { data: tree, isLoading, isError } = useQuery({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => unwrap(apiClient.GET('/api/explorer/tree', {})),
    refetchInterval: 30_000,
  });

  if (isLoading) return <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>Loading…</div>;
  if (isError) return <div data-testid="error-state" style={{ padding: 24, color: 'var(--err)', fontSize: 13 }}>Failed to load domains.</div>;
  if (!tree || tree.domains.length === 0) {
    return <div data-testid="empty-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>No domains.</div>;
  }

  return (
    <div data-testid="domains-overview" style={{ padding: '16px 24px' }}>
      <div style={{ marginBottom: 16 }}>
        <span style={{ fontSize: 11, fontWeight: 590, color: 'var(--dim)', letterSpacing: '0.06em', textTransform: 'uppercase' }}>
          {tree.domain_count} domains · {tree.mission_count} missions · {tree.task_count} tasks
        </span>
      </div>
      {tree.domains.map((domain) => (
        <DomainOverviewRow key={domain.id} domain={domain} />
      ))}
    </div>
  );
}

function statusDot(status: string): string {
  const v = status.toLowerCase();
  if (v === 'done' || v === 'completed') return '✓';
  if (v === 'in_progress' || v === 'running') return '⟳';
  if (v === 'blocked' || v === 'failed') return '✗';
  if (v === 'proposed') return '○';
  return '●';
}

function statusColor(status: string): string {
  const v = status.toLowerCase();
  if (v === 'done' || v === 'completed') return 'var(--ok)';
  if (v === 'blocked' || v === 'failed') return 'var(--err)';
  if (v === 'in_progress' || v === 'active' || v === 'running') return 'var(--accent)';
  return 'var(--dim)';
}

function DomainOverviewRow({ domain }: { domain: ExplorerDomainNode }) {
  const [expanded, setExpanded] = useState(true);

  return (
    <div style={{ marginBottom: 8 }}>
      <div
        data-testid={`domain-row-${domain.id}`}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '6px 8px',
          borderRadius: 6,
          cursor: 'pointer',
          background: 'var(--raised)',
          marginBottom: expanded ? 2 : 0,
        }}
      >
        <button
          type="button"
          aria-label={expanded ? 'Collapse' : 'Expand'}
          onClick={() => setExpanded((v) => !v)}
          style={{ background: 'none', border: 'none', color: 'var(--dim)', fontSize: 11, cursor: 'pointer', padding: 0, width: 16 }}
        >
          {expanded ? '▾' : '▸'}
        </button>
        <Link
          to="/domains/$domainId"
          params={{ domainId: domain.id }}
          style={{ flex: 1, display: 'flex', alignItems: 'center', gap: 8, textDecoration: 'none' }}
        >
          <span style={{ fontSize: 13, fontWeight: 510, color: 'var(--text)' }}>{domain.name}</span>
          <span style={{ fontSize: 11, color: statusColor(domain.status), background: 'var(--raised-2)', padding: '1px 6px', borderRadius: 3 }}>
            {domain.status}
          </span>
          <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)' }}>
            {domain.mission_count}m · {domain.task_count}t
          </span>
        </Link>
      </div>
      {expanded && domain.missions.map((mission) => (
        <MissionOverviewRow key={mission.id} mission={mission} domainId={domain.id} />
      ))}
    </div>
  );
}

function MissionOverviewRow({ mission, domainId }: { mission: ExplorerMissionNode; domainId: string }) {
  return (
    <div data-testid={`mission-row-${mission.id}`} style={{ marginBottom: 1 }}>
      <Link
        to="/domains/$domainId/missions/$missionId"
        params={{ domainId, missionId: mission.id }}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '4px 8px 4px 32px',
          borderRadius: 5,
          textDecoration: 'none',
          color: 'var(--text-2)',
          fontSize: 12,
        }}
      >
        <span style={{ color: statusColor(mission.status) }}>{statusDot(mission.status)}</span>
        <span>{mission.name}</span>
        <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)' }}>{mission.task_count}t</span>
      </Link>
    </div>
  );
}
