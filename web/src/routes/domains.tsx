/**
 * Domains screen (formerly Explorer) — Phase 2 React migration.
 *
 * Data sources:
 *   - GET /api/explorer/tree       (typed — ExplorerTreeResponse)
 *   - GET /api/explorer/node/{node_type}/{node_id}  (typed — ExplorerNodeDetail)
 *
 * Layout: 3-pane (domains list | details pane | agent terminal stub)
 * Cadence: tree refetchInterval 30s (matches Svelte page)
 *
 * Svelte parity: web/src/routes/explorer/+page.svelte
 *   - Domain list with nested missions, mission task count, expand-in-place
 *   - Click domain → inline detail (no API call, from tree data)
 *   - Click mission → fetch node detail via API
 *   - Click task → fetch node detail via API (from mission detail task list)
 *   - Status dots: ✓ done/completed, ⟳ in_progress/running, ✗ blocked/failed,
 *                  ○ proposed, ● other
 *   - Search filter across domain names
 *   - Refresh button + last-refreshed timestamp
 *   - Agent terminal pane (stub — ACP wiring out of scope for Phase 2)
 *   - Loading/error/empty states for both tree and node detail
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';

// ── Generated schema types ─────────────────────────────────────────────────────

type ExplorerTreeResponse = components['schemas']['ExplorerTreeResponse'];
type ExplorerDomainNode = components['schemas']['ExplorerDomainNode'];
type ExplorerMissionNode = components['schemas']['ExplorerMissionNode'];
type ExplorerNodeDetail = components['schemas']['ExplorerNodeDetail'];
type ExplorerTask = components['schemas']['ExplorerTask'];

// ── Route ──────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/domains')({
  component: ExplorerPage,
});

// ── Selection state ────────────────────────────────────────────────────────────

/** For domains: inline detail from tree data (no API call needed).
 *  For missions and tasks: fetch via ExplorerNodeDetail API. */
type SelectionKey = { type: 'domain' | 'mission' | 'task'; id: string } | null;

// ── Helpers ───────────────────────────────────────────────────────────────────

function statusDot(status?: string | null): string {
  const v = String(status ?? '').toLowerCase();
  if (v === 'done' || v === 'completed') return '✓';
  if (v === 'in_progress' || v === 'running') return '⟳';
  if (v === 'blocked' || v === 'failed') return '✗';
  if (v === 'proposed') return '○';
  return '●';
}

function statusTagVariant(status?: string | null): 'ok' | 'warn' | 'err' | 'accent' | 'default' {
  const v = String(status ?? '').toLowerCase();
  if (v === 'done' || v === 'completed') return 'ok';
  if (v === 'blocked' || v === 'failed') return 'err';
  if (v === 'in_progress' || v === 'running') return 'accent';
  if (v === 'proposed') return 'default';
  return 'default';
}

function fmtDate(s: string | null | undefined): string {
  if (!s) return '—';
  return new Date(s).toLocaleString();
}

function taskCountByStatus(tasks: ExplorerTask[], status: string): number {
  return tasks.filter((t) => String(t.status ?? '').toLowerCase() === status).length;
}

// ── Inline components ─────────────────────────────────────────────────────────

function Tag({
  variant = 'default',
  children,
}: {
  variant?: 'ok' | 'warn' | 'err' | 'accent' | 'purple' | 'default';
  children: React.ReactNode;
}) {
  return <span className={`tag ${variant !== 'default' ? variant : ''}`}>{children}</span>;
}

// ── Tree pane sub-components ──────────────────────────────────────────────────

// Base button style — resets button appearance for .tnode rows
const tnodeBtnBase: React.CSSProperties = {
  width: '100%',
  background: 'none',
  border: 'none',
  textAlign: 'left',
  cursor: 'pointer',
  font: 'inherit',
};

// Shared row button style — used in detail pane sub-components
const rowBtnStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  width: '100%',
  background: 'none',
  border: 'none',
  padding: '4px 8px',
  textAlign: 'left',
  cursor: 'pointer',
  font: 'inherit',
  color: 'inherit',
};

function MissionRow({
  mission,
  selected,
  onSelect,
}: {
  mission: ExplorerMissionNode;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      className={`tnode lvl1${selected ? ' active' : ''}`}
      style={{
        ...tnodeBtnBase,
        background: selected ? 'var(--raised-2)' : undefined,
      }}
      onClick={onSelect}
      data-testid={`mission-row-${mission.id}`}
    >
      <span className="tw" style={{ color: 'var(--dim)', width: '12px' }}>
        ▸
      </span>
      <span
        className="name"
        style={{
          flex: 1,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {mission.name}
      </span>
      <span
        className="count"
        style={{
          marginLeft: 'auto',
          fontSize: '11px',
          color: 'var(--dim)',
          fontFamily: 'var(--mono)',
        }}
      >
        {mission.task_count}t
      </span>
    </button>
  );
}

function DomainRow({
  domain,
  selected,
  selectedMissionId,
  onSelectDomain,
  onSelectMission,
}: {
  domain: ExplorerDomainNode;
  selected: boolean;
  selectedMissionId: string | null;
  onSelectDomain: () => void;
  onSelectMission: (m: ExplorerMissionNode) => void;
}) {
  return (
    <>
      <button
        type="button"
        className={`tnode${selected ? ' active' : ''}`}
        style={{
          ...tnodeBtnBase,
          background: selected ? 'var(--raised-2)' : undefined,
        }}
        onClick={onSelectDomain}
        data-testid={`domain-row-${domain.id}`}
      >
        <span className="tw" style={{ color: 'var(--dim)', width: '12px' }}>
          ▾
        </span>
        <span
          className="name"
          style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
        >
          {domain.name}
        </span>
        <span
          className="count"
          style={{
            marginLeft: 'auto',
            fontSize: '11px',
            color: 'var(--dim)',
            fontFamily: 'var(--mono)',
          }}
        >
          {domain.missions.length}m
        </span>
      </button>
      {domain.missions.map((m) => (
        <MissionRow
          key={m.id}
          mission={m}
          selected={selectedMissionId === m.id}
          onSelect={() => onSelectMission(m)}
        />
      ))}
    </>
  );
}

// ── Detail pane sub-components ────────────────────────────────────────────────

function DomainDetail({
  domain,
  onSelectMission,
}: {
  domain: ExplorerDomainNode;
  onSelectMission: (m: ExplorerMissionNode) => void;
}) {
  const variant = statusTagVariant(domain.status);
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '8px' }}>
        <strong>{domain.name}</strong>
        <Tag variant={variant}>{domain.status ?? 'unknown'}</Tag>
      </div>
      <p className="muted" style={{ margin: '0 0 10px', fontSize: '12px' }}>
        {domain.description || 'No description.'}
      </p>
      <div
        style={{
          display: 'flex',
          gap: '12px',
          fontSize: '11px',
          color: 'var(--dim)',
          marginBottom: '10px',
        }}
      >
        <span>
          Missions: <span style={{ color: 'var(--text)' }}>{domain.mission_count}</span>
        </span>
        <span>
          Tasks: <span style={{ color: 'var(--text)' }}>{domain.task_count}</span>
        </span>
      </div>
      {domain.missions.length > 0 && (
        <div>
          <p className="section-label">Missions</p>
          {domain.missions.map((m) => (
            <button
              key={m.id}
              type="button"
              className="row"
              style={rowBtnStyle}
              onClick={() => onSelectMission(m)}
            >
              <span className="dim">▸</span>
              <span style={{ flex: 1, fontSize: '12px' }}>{m.name}</span>
              <Tag variant={statusTagVariant(m.status)}>{m.status}</Tag>
              <span className="dim" style={{ fontSize: '10px', marginLeft: '4px' }}>
                {m.task_count}t
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function MissionDetail({
  detail,
  onSelectTask,
}: {
  detail: ExplorerNodeDetail;
  onSelectTask: (taskId: string) => void;
}) {
  const m = detail.mission;
  const tasks = (detail.tasks ?? []) as ExplorerTask[];
  const variant = statusTagVariant(m?.status);

  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '8px' }}>
        <strong>{m?.name ?? 'Mission'}</strong>
        <Tag variant={variant}>{m?.status ?? 'unknown'}</Tag>
      </div>
      <p className="muted" style={{ margin: '0 0 10px', fontSize: '12px' }}>
        {m?.description || 'No description.'}
      </p>
      <div
        style={{
          display: 'flex',
          gap: '12px',
          fontSize: '11px',
          color: 'var(--dim)',
          marginBottom: '10px',
        }}
      >
        <span>Tasks: {tasks.length}</span>
        <span>In progress: {taskCountByStatus(tasks, 'in_progress')}</span>
        <span>Blocked: {taskCountByStatus(tasks, 'blocked')}</span>
      </div>
      {tasks.length > 0 ? (
        tasks.map((t) => (
          <div key={t.id} style={{ padding: '7px 0', borderBottom: '1px solid var(--border)' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
              <span>{statusDot(t.status)}</span>
              <strong style={{ fontSize: '12px', flex: 1 }}>{t.title}</strong>
              <Tag variant={statusTagVariant(t.status)}>{t.status ?? 'unknown'}</Tag>
            </div>
            <p className="muted" style={{ margin: '3px 0 0', fontSize: '11px' }}>
              {t.description || 'No description.'}
            </p>
            <button
              type="button"
              className="ghost"
              style={{ marginTop: '5px', fontSize: '11px' }}
              onClick={() => onSelectTask(String(t.public_id ?? t.id))}
              data-testid={`open-task-${t.id}`}
            >
              Open Task
            </button>
          </div>
        ))
      ) : (
        <p className="muted" style={{ fontSize: '12px' }}>
          No sub-tasks.
        </p>
      )}
    </div>
  );
}

function TaskDetail({ detail }: { detail: ExplorerNodeDetail }) {
  const t = detail.task;
  const variant = statusTagVariant(t?.status);
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '8px' }}>
        <strong>{t?.title ?? 'Task'}</strong>
        <Tag variant={variant}>{t?.status ?? 'unknown'}</Tag>
      </div>
      <p className="muted" style={{ margin: '0 0 10px', fontSize: '12px' }}>
        {t?.description || 'No description.'}
      </p>
      {t && (
        <dl className="policy-meta">
          <dt>Public ID</dt>
          <dd style={{ fontFamily: 'var(--mono)', fontSize: '11px', color: 'var(--accent)' }}>
            {t.public_id}
          </dd>
          <dt>Owner</dt>
          <dd>{t.owner || '—'}</dd>
          <dt>Contributors</dt>
          <dd>{t.contributors || '—'}</dd>
          <dt>Mission</dt>
          <dd style={{ fontSize: '11px', color: 'var(--dim)' }}>{t.mission_id}</dd>
          <dt>Created</dt>
          <dd>{fmtDate(t.created_at)}</dd>
          <dt>Updated</dt>
          <dd>{fmtDate(t.updated_at)}</dd>
        </dl>
      )}
      <details style={{ marginTop: '12px' }}>
        <summary style={{ fontSize: '11px', color: 'var(--dim)', cursor: 'pointer' }}>
          Raw JSON
        </summary>
        <pre style={{ fontSize: '11px', marginTop: '6px' }}>{JSON.stringify(detail, null, 2)}</pre>
      </details>
    </div>
  );
}

// ── Unassigned missions ────────────────────────────────────────────────────────

function UnassignedMissions({
  missions,
  selectedId,
  onSelect,
}: {
  missions: ExplorerMissionNode[];
  selectedId: string | null;
  onSelect: (m: ExplorerMissionNode) => void;
}) {
  if (missions.length === 0) return null;
  return (
    <>
      <div
        className="tnode"
        style={{
          color: 'var(--dim)',
          fontSize: '10px',
          textTransform: 'uppercase',
          cursor: 'default',
        }}
      >
        Unassigned
      </div>
      {missions.map((m) => (
        <MissionRow
          key={m.id}
          mission={m}
          selected={selectedId === m.id}
          onSelect={() => onSelect(m)}
        />
      ))}
    </>
  );
}

// ── Main page ──────────────────────────────────────────────────────────────────

// Named export for direct use in tests (avoids router context requirement)
export function ExplorerPage() {
  const queryClient = useQueryClient();

  const [searchInput, setSearchInput] = useState('');
  const [selection, setSelection] = useState<SelectionKey>(null);

  // Track domain selection separately (inline, no API call)
  const [selectedDomainNode, setSelectedDomainNode] = useState<ExplorerDomainNode | null>(null);

  // ── Tree query ─────────────────────────────────────────────────────────────
  const treeQuery = useQuery<ExplorerTreeResponse>({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => unwrap(apiClient.GET('/api/explorer/tree')),
    refetchInterval: 30_000,
  });

  // ── Node detail query ──────────────────────────────────────────────────────
  // Only enabled when a mission or task is selected (domains use inline data)
  const nodeQuery = useQuery<ExplorerNodeDetail>({
    queryKey: selection
      ? queryKeys.explorer.node(selection.type, selection.id)
      : (['__explorer_none__'] as const),
    queryFn: () =>
      unwrap(
        apiClient.GET('/api/explorer/node/{node_type}/{node_id}', {
          params: { path: { node_type: selection?.type ?? '', node_id: selection?.id ?? '' } },
        }),
      ),
    enabled: !!selection && selection.type !== 'domain',
    refetchOnWindowFocus: false,
  });

  // ── Derived ────────────────────────────────────────────────────────────────
  const tree = treeQuery.data;
  const domains = tree?.domains ?? [];
  const unassigned = tree?.unassigned_missions ?? [];

  const filteredDomains = searchInput
    ? domains.filter((d) => d.name.toLowerCase().includes(searchInput.toLowerCase()))
    : domains;

  const lastRefreshed =
    treeQuery.dataUpdatedAt > 0 ? new Date(treeQuery.dataUpdatedAt).toLocaleTimeString() : '';

  // ── Selection handlers ─────────────────────────────────────────────────────

  function selectDomain(d: ExplorerDomainNode) {
    setSelectedDomainNode(d);
    setSelection(null); // domain details come from tree data, no API call
  }

  function selectMission(m: ExplorerMissionNode) {
    setSelectedDomainNode(null);
    setSelection({ type: 'mission', id: m.id });
  }

  function selectTask(taskId: string) {
    setSelectedDomainNode(null);
    setSelection({ type: 'task', id: taskId });
  }

  function refreshTree() {
    queryClient.invalidateQueries({ queryKey: queryKeys.explorer.tree() });
  }

  // ── Determine selected mission id for highlighting ─────────────────────────
  const selectedMissionId = selection?.type === 'mission' ? selection.id : null;
  const selectedDomainId = selectedDomainNode?.id ?? null;

  // ── Render ─────────────────────────────────────────────────────────────────

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}
      data-testid="explorer-page"
    >
      {/* Top filter bar */}
      <div className="gov-bar">
        <input
          value={searchInput}
          onChange={(e) => setSearchInput(e.target.value)}
          placeholder="filter domains…"
          style={{ width: '180px' }}
          data-testid="search-input"
        />
        <button type="button" className="ghost" onClick={refreshTree} data-testid="refresh-btn">
          Refresh
        </button>
        {lastRefreshed && (
          <span className="muted" style={{ fontSize: '11px' }}>
            updated {lastRefreshed}
          </span>
        )}
      </div>

      {/* Tree loading */}
      {treeQuery.isLoading && (
        <div style={{ padding: '12px' }}>
          <p className="muted" data-testid="loading-state">
            ⟳ Loading explorer…
          </p>
        </div>
      )}

      {/* Tree error */}
      {treeQuery.isError && (
        <div style={{ padding: '12px' }}>
          <p className="error" data-testid="error-state">
            ✗ Failed to load explorer — {(treeQuery.error as Error)?.message ?? 'unknown error'}
          </p>
        </div>
      )}

      {/* 3-pane layout — shown once tree resolves with data */}
      {!treeQuery.isLoading &&
        !treeQuery.isError &&
        (domains.length > 0 || unassigned.length > 0) && (
          <div className="pane-row" style={{ flex: 1, minHeight: 0 }}>
            {/* Pane 1: Domains + missions tree (Linear density) */}
            <div
              className="pane"
              style={{
                width: '260px',
                flexShrink: 0,
                display: 'flex',
                flexDirection: 'column',
              }}
            >
              <div className="pane-header">
                <span className="pane-title">Domains</span>
                <span className="dim">{filteredDomains.length}</span>
              </div>
              <div
                className="tree"
                style={{
                  flex: 1,
                  overflowY: 'auto',
                  padding: '4px 8px',
                  fontSize: '13px',
                }}
              >
                {filteredDomains.length > 0 ? (
                  filteredDomains.map((d) => (
                    <DomainRow
                      key={d.id}
                      domain={d}
                      selected={selectedDomainId === d.id}
                      selectedMissionId={selectedMissionId}
                      onSelectDomain={() => selectDomain(d)}
                      onSelectMission={selectMission}
                    />
                  ))
                ) : (
                  <div
                    className="tnode"
                    style={{ color: 'var(--muted)', fontSize: '12px', cursor: 'default' }}
                  >
                    No domains yet.
                  </div>
                )}
                <UnassignedMissions
                  missions={unassigned}
                  selectedId={selectedMissionId}
                  onSelect={selectMission}
                />
              </div>
            </div>

            {/* Pane 2: Detail */}
            <div className="pane" style={{ flex: 1, minWidth: 0 }}>
              <div className="pane-header">
                <span className="pane-title">Details</span>
                {nodeQuery.isFetching && (
                  <span className="muted" style={{ fontSize: '10px' }}>
                    ⟳
                  </span>
                )}
              </div>
              <div className="pane-body" style={{ padding: '10px', overflow: 'auto' }}>
                {/* Node detail loading */}
                {nodeQuery.isLoading && selection && selection.type !== 'domain' && (
                  <p className="muted" data-testid="detail-loading-state">
                    ⟳ Loading…
                  </p>
                )}

                {/* Node detail error */}
                {nodeQuery.isError && (
                  <p className="error" data-testid="detail-error-state">
                    ✗ Failed to load detail —{' '}
                    {(nodeQuery.error as Error)?.message ?? 'unknown error'}
                  </p>
                )}

                {/* Domain detail (inline from tree) */}
                {!nodeQuery.isLoading && selectedDomainNode && !selection && (
                  <DomainDetail domain={selectedDomainNode} onSelectMission={selectMission} />
                )}

                {/* Mission or task detail (from API) */}
                {!nodeQuery.isLoading && !nodeQuery.isError && nodeQuery.data && (
                  <>
                    {nodeQuery.data.node_type === 'mission' && (
                      <MissionDetail detail={nodeQuery.data} onSelectTask={selectTask} />
                    )}
                    {nodeQuery.data.node_type === 'task' && <TaskDetail detail={nodeQuery.data} />}
                    {nodeQuery.data.node_type !== 'mission' &&
                      nodeQuery.data.node_type !== 'task' && (
                        <pre style={{ fontSize: '11px' }}>
                          {JSON.stringify(nodeQuery.data, null, 2)}
                        </pre>
                      )}
                  </>
                )}

                {/* Prompt when nothing selected */}
                {!nodeQuery.isLoading && !selectedDomainNode && !selection && (
                  <p className="muted" data-testid="detail-empty-state">
                    Select a domain or mission.
                  </p>
                )}
              </div>
            </div>

            {/* Pane 3: Agent terminal stub */}
            <div className="pane" style={{ width: '320px', flexShrink: 0 }}>
              <div className="pane-header">
                <span className="pane-title">Agent Terminal</span>
                <span className="muted" style={{ fontSize: '10px' }}>
                  Phase 3
                </span>
              </div>
              <div className="pane-body">
                <div
                  style={{ padding: '10px', display: 'flex', flexDirection: 'column', gap: '6px' }}
                >
                  <input placeholder="node id (e.g. epyc)" />
                  <input placeholder="agent id" />
                  <button type="button" className="ghost" disabled style={{ fontSize: '11px' }}>
                    Attach
                  </button>
                  <p className="muted" style={{ fontSize: '11px', marginTop: '4px' }}>
                    ACP terminal attach coming in Phase 3.
                  </p>
                </div>
              </div>
            </div>
          </div>
        )}

      {/* Full empty state when tree resolves but has no domains */}
      {!treeQuery.isLoading &&
        !treeQuery.isError &&
        domains.length === 0 &&
        unassigned.length === 0 && (
          <div className="empty-state" data-testid="empty-state">
            <div className="empty-icon">⊙</div>
            <div className="empty-title">No domains or missions</div>
            <div className="empty-body">
              Create a domain with <code>edgeplane domain create</code> to get started.
            </div>
          </div>
        )}
    </div>
  );
}
