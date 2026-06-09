import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { NarrativeEditor } from '@/components/domains/NarrativeEditor';
import { queryKeys } from '@/lib/queryKeys';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, createFileRoute, useParams } from '@tanstack/react-router';
import { useState } from 'react';

type ExplorerDomainNode = components['schemas']['ExplorerDomainNode'];
type ExplorerMissionNode = components['schemas']['ExplorerMissionNode'];

interface NorthstarResponse {
  northstar_md: string;
  northstar_version: number;
  northstar_modified_by: string | null;
  northstar_modified_at: string | null;
}

export const Route = createFileRoute('/domains/$domainId')({
  component: DomainPage,
});

export function DomainPage() {
  const { domainId } = useParams({ from: '/domains/$domainId' });
  const [activeTab, setActiveTab] = useState<'northstar' | 'missions' | 'overview'>('northstar');
  const qc = useQueryClient();

  const { data: tree, isLoading: treeLoading } = useQuery({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => unwrap(apiClient.GET('/api/explorer/tree', {})),
    refetchInterval: 30_000,
  });

  const domain: ExplorerDomainNode | undefined = tree?.domains?.find((d) => d.id === domainId);

  const { data: northstar, isLoading: nsLoading } = useQuery({
    queryKey: queryKeys.domains.northstar(domainId),
    queryFn: async (): Promise<NorthstarResponse> => {
      const res = await fetch(`/api/domains/${domainId}/northstar`, {
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
      });
      if (!res.ok) throw new Error(`northstar fetch failed: ${res.status}`);
      return res.json();
    },
    enabled: activeTab === 'northstar',
  });

  const saveMutation = useMutation({
    mutationFn: async (md: string) => {
      const res = await fetch(`/api/domains/${domainId}/northstar`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({ northstar_md: md }),
      });
      if (!res.ok) throw new Error(`save failed: ${res.status}`);
      return res.json();
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.domains.northstar(domainId) });
    },
  });

  if (treeLoading)
    return (
      <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>
        Loading…
      </div>
    );
  if (!domain)
    return (
      <div data-testid="not-found-state" style={{ padding: 24, color: 'var(--err)', fontSize: 13 }}>
        Domain not found.
      </div>
    );

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      {/* Header */}
      <div
        data-testid="domain-header"
        style={{ padding: '12px 24px 0', borderBottom: '1px solid var(--border)', flexShrink: 0 }}
      >
        <div style={{ fontSize: 11, color: 'var(--dim)', marginBottom: 4 }}>
          <Link to="/domains" style={{ color: 'var(--dim)', textDecoration: 'none' }}>
            {'Domains › '}
            {domain.name}
          </Link>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
          <span style={{ fontSize: 16, fontWeight: 590, color: 'var(--text)' }}>{domain.name}</span>
          <span
            style={{
              fontSize: 11,
              color: 'var(--accent)',
              background: 'var(--accent-dim)',
              padding: '1px 6px',
              borderRadius: 3,
            }}
          >
            {domain.status}
          </span>
          <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)' }}>
            {domain.mission_count} missions · {domain.task_count} tasks
          </span>
        </div>
        {/* Tabs */}
        <div style={{ display: 'flex', gap: 0 }}>
          {(['northstar', 'missions', 'overview'] as const).map((tab) => (
            <button
              key={tab}
              type="button"
              data-testid={`tab-${tab}`}
              aria-selected={activeTab === tab}
              onClick={() => setActiveTab(tab)}
              style={{
                padding: '6px 14px',
                fontSize: 12,
                background: 'none',
                border: 'none',
                borderBottom:
                  activeTab === tab ? '2px solid var(--accent)' : '2px solid transparent',
                color: activeTab === tab ? 'var(--text)' : 'var(--dim)',
                cursor: 'pointer',
                fontFamily: 'var(--font)',
                textTransform: 'capitalize',
                marginBottom: -1,
              }}
            >
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>
      </div>

      {/* Tab content */}
      <div
        style={{
          flex: 1,
          minHeight: 0,
          padding: '16px 24px',
          display: 'flex',
          flexDirection: 'column',
        }}
      >
        {activeTab === 'northstar' &&
          (nsLoading ? (
            <div data-testid="northstar-loading" style={{ color: 'var(--dim)', fontSize: 13 }}>
              Loading…
            </div>
          ) : (
            <NarrativeEditor
              key={domainId}
              initialValue={northstar?.northstar_md ?? ''}
              onSave={(md) => saveMutation.mutate(md)}
              isSaving={saveMutation.isPending}
              saveError={saveMutation.isError ? String(saveMutation.error) : null}
              version={northstar?.northstar_version}
              modifiedAt={northstar?.northstar_modified_at}
            />
          ))}
        {activeTab === 'missions' && <MissionsTab missions={domain.missions} domainId={domainId} />}
        {activeTab === 'overview' && <OverviewTab domain={domain} />}
      </div>
    </div>
  );
}

function MissionsTab({
  missions,
  domainId,
}: { missions: ExplorerMissionNode[]; domainId: string }) {
  if (missions.length === 0) {
    return <div style={{ color: 'var(--dim)', fontSize: 13 }}>No missions in this domain.</div>;
  }
  return (
    <div>
      {missions.map((m) => (
        <Link
          key={m.id}
          to="/domains/$domainId/missions/$missionId"
          params={{ domainId, missionId: m.id }}
          data-testid={`mission-row-${m.id}`}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: '8px 10px',
            borderRadius: 5,
            textDecoration: 'none',
            marginBottom: 2,
            background: 'var(--raised)',
          }}
        >
          <span style={{ fontSize: 13, color: 'var(--text)' }}>{m.name}</span>
          <span
            style={{
              fontSize: 11,
              color: 'var(--accent)',
              background: 'var(--accent-dim)',
              padding: '1px 5px',
              borderRadius: 3,
            }}
          >
            {m.status}
          </span>
          <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)' }}>
            {m.task_count}t
          </span>
        </Link>
      ))}
    </div>
  );
}

function OverviewTab({ domain }: { domain: ExplorerDomainNode }) {
  return (
    <dl
      style={{
        display: 'grid',
        gridTemplateColumns: 'max-content 1fr',
        gap: '6px 16px',
        fontSize: 13,
      }}
    >
      <dt style={{ color: 'var(--dim)' }}>Description</dt>
      <dd style={{ color: 'var(--text)', margin: 0 }}>{domain.description || '—'}</dd>
      <dt style={{ color: 'var(--dim)' }}>Status</dt>
      <dd style={{ color: 'var(--accent)', margin: 0 }}>{domain.status}</dd>
      <dt style={{ color: 'var(--dim)' }}>Owners</dt>
      <dd style={{ color: 'var(--text)', margin: 0 }}>{domain.owners || '—'}</dd>
      <dt style={{ color: 'var(--dim)' }}>Updated</dt>
      <dd style={{ color: 'var(--text)', margin: 0 }}>
        {new Date(domain.updated_at).toLocaleString()}
      </dd>
    </dl>
  );
}
