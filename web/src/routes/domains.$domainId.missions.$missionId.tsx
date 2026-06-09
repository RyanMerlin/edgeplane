import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { NarrativeEditor } from '@/components/domains/NarrativeEditor';
import { TaskSlideOver } from '@/components/domains/TaskSlideOver';
import { queryKeys } from '@/lib/queryKeys';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, createFileRoute, useParams } from '@tanstack/react-router';
import { useState } from 'react';

type ExplorerMission = components['schemas']['ExplorerMission'];

interface BriefResponse {
  brief_md: string;
  brief_version: number;
  brief_modified_by: string | null;
  brief_modified_at: string | null;
}

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

export const Route = createFileRoute('/domains/$domainId/missions/$missionId')({
  component: MissionPage,
});

export function MissionPage() {
  const { domainId, missionId } = useParams({ from: '/domains/$domainId/missions/$missionId' });
  const [activeTab, setActiveTab] = useState<'brief' | 'tasks' | 'overview'>('brief');
  const [selectedTask, setSelectedTask] = useState<TaskRecord | null>(null);
  const qc = useQueryClient();

  const { data: detail, isLoading } = useQuery({
    queryKey: queryKeys.explorer.node('mission', missionId),
    queryFn: () =>
      unwrap(
        apiClient.GET('/api/explorer/node/{node_type}/{node_id}', {
          params: { path: { node_type: 'mission', node_id: missionId } },
        }),
      ),
  });

  const mission = detail?.mission as ExplorerMission | undefined;
  const tasks = (detail?.tasks ?? []) as TaskRecord[];

  const { data: brief, isLoading: briefLoading } = useQuery({
    queryKey: queryKeys.domains.brief(domainId, missionId),
    queryFn: async (): Promise<BriefResponse> => {
      const res = await fetch(`/api/domains/${domainId}/m/${missionId}/brief`, {
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
      });
      if (!res.ok) throw new Error(`brief fetch failed: ${res.status}`);
      return res.json();
    },
    enabled: activeTab === 'brief',
  });

  const saveMutation = useMutation({
    mutationFn: async (md: string) => {
      const res = await fetch(`/api/domains/${domainId}/m/${missionId}/brief`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({ brief_md: md }),
      });
      if (!res.ok) throw new Error(`save failed: ${res.status}`);
      return res.json();
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.domains.brief(domainId, missionId) });
    },
  });

  if (isLoading)
    return (
      <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>
        Loading…
      </div>
    );
  if (!mission)
    return (
      <div data-testid="not-found-state" style={{ padding: 24, color: 'var(--err)', fontSize: 13 }}>
        Mission not found.
      </div>
    );

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div
        data-testid="mission-header"
        style={{ padding: '12px 24px 0', borderBottom: '1px solid var(--border)', flexShrink: 0 }}
      >
        <div style={{ fontSize: 11, color: 'var(--dim)', marginBottom: 4 }}>
          <Link to="/domains" style={{ color: 'var(--dim)', textDecoration: 'none' }}>
            Domains
          </Link>
          {' › '}
          <Link
            to="/domains/$domainId"
            params={{ domainId }}
            style={{ color: 'var(--dim)', textDecoration: 'none' }}
          >
            {domainId}
          </Link>
          {' › '}
          <span style={{ color: 'var(--text)' }}>{mission.name}</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
          <span style={{ fontSize: 16, fontWeight: 590, color: 'var(--text)' }}>
            {mission.name}
          </span>
          <span
            style={{
              fontSize: 11,
              color: 'var(--accent)',
              background: 'var(--accent-dim)',
              padding: '1px 6px',
              borderRadius: 3,
            }}
          >
            {mission.status}
          </span>
        </div>
        <div style={{ display: 'flex', gap: 0 }}>
          {(['brief', 'tasks', 'overview'] as const).map((tab) => (
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

      <div
        style={{
          flex: 1,
          minHeight: 0,
          padding: '16px 24px',
          display: 'flex',
          flexDirection: 'column',
        }}
      >
        {activeTab === 'brief' &&
          (briefLoading ? (
            <div data-testid="brief-loading" style={{ color: 'var(--dim)', fontSize: 13 }}>
              Loading…
            </div>
          ) : (
            <NarrativeEditor
              key={missionId}
              initialValue={brief?.brief_md ?? ''}
              onSave={(md) => saveMutation.mutate(md)}
              isSaving={saveMutation.isPending}
              saveError={saveMutation.isError ? String(saveMutation.error) : null}
              version={brief?.brief_version}
              modifiedAt={brief?.brief_modified_at}
            />
          ))}
        {activeTab === 'tasks' && (
          <TasksTab tasks={tasks} onTaskClick={(t) => setSelectedTask(t)} />
        )}
        {activeTab === 'overview' && <OverviewTab mission={mission} />}
      </div>

      <TaskSlideOver
        task={selectedTask}
        isOpen={selectedTask !== null}
        onClose={() => setSelectedTask(null)}
      />
    </div>
  );
}

function TasksTab({
  tasks,
  onTaskClick,
}: { tasks: TaskRecord[]; onTaskClick: (t: TaskRecord) => void }) {
  if (tasks.length === 0) {
    return <div style={{ color: 'var(--dim)', fontSize: 13 }}>No tasks in this mission.</div>;
  }
  return (
    <div>
      {tasks.map((t) => (
        <button
          key={t.id}
          type="button"
          data-testid={`task-row-${t.id}`}
          onClick={() => onTaskClick(t)}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: '8px 10px',
            borderRadius: 5,
            marginBottom: 2,
            width: '100%',
            textAlign: 'left',
            background: 'var(--raised)',
            border: 'none',
            cursor: 'pointer',
            fontFamily: 'var(--font)',
          }}
        >
          <span style={{ fontSize: 13, color: 'var(--text)', flex: 1 }}>{t.title}</span>
          <span
            style={{
              fontSize: 11,
              color: 'var(--accent)',
              background: 'var(--accent-dim)',
              padding: '1px 5px',
              borderRadius: 3,
            }}
          >
            {t.status}
          </span>
          {t.owner && <span style={{ fontSize: 11, color: 'var(--dim)' }}>{t.owner}</span>}
        </button>
      ))}
    </div>
  );
}

function OverviewTab({ mission }: { mission: ExplorerMission }) {
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
      <dd style={{ margin: 0, color: 'var(--text)' }}>{mission.description || '—'}</dd>
      <dt style={{ color: 'var(--dim)' }}>Status</dt>
      <dd style={{ margin: 0, color: 'var(--accent)' }}>{mission.status}</dd>
      <dt style={{ color: 'var(--dim)' }}>Owners</dt>
      <dd style={{ margin: 0, color: 'var(--text)' }}>{mission.owners || '—'}</dd>
      <dt style={{ color: 'var(--dim)' }}>Updated</dt>
      <dd style={{ margin: 0, color: 'var(--text)' }}>
        {new Date(mission.updated_at).toLocaleString()}
      </dd>
    </dl>
  );
}
