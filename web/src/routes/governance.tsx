/**
 * Governance screen — Phase 0.9 frontend slice.
 *
 * Data source:  GET  /api/governance/policy/active (typed via schema.gen.ts)
 * Mutation:     POST /api/governance/policy/reload  (typed via schema.gen.ts)
 * Events feed:  GET  /api/governance/policy/events  (typed via schema.gen.ts)
 * Cadence:      refetchInterval 30s (matches Svelte page)
 *
 * `policy` is now a real typed object from the generated schema — the local
 * PolicyDoc cast / `unknown` workaround has been removed.
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useToastStore } from '@/stores/toast';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';

// ── Generated schema types (no local casts needed) ────────────────────────────

type PolicyRecord = components['schemas']['GovernancePolicyResponse'];
type PolicyActionRule = components['schemas']['PolicyActionRule'];
type PolicyEvent = components['schemas']['PolicyEvent'];

// ── Route ─────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/governance')({
  component: GovernancePage,
});

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtDate(s: string | null | undefined): string {
  if (!s) return '—';
  return new Date(s).toLocaleString();
}

function stateVariant(state?: string): 'ok' | 'warn' | 'err' | 'default' {
  if (state === 'active') return 'ok';
  if (state === 'draft') return 'warn';
  if (state === 'archived') return 'err';
  return 'default';
}

// Inline tag — mirrors the `.tag` CSS class from app.css
function Tag({
  variant = 'default',
  children,
}: {
  variant?: 'ok' | 'warn' | 'err' | 'accent' | 'purple' | 'default';
  children: React.ReactNode;
}) {
  return <span className={`tag ${variant !== 'default' ? variant : ''}`}>{children}</span>;
}

// ── Sub-components ────────────────────────────────────────────────────────────

function PolicyMeta({ policy }: { policy: PolicyRecord }) {
  return (
    <dl className="policy-meta">
      <dt>Published by</dt>
      <dd>{policy.published_by || '—'}</dd>
      <dt>Published at</dt>
      <dd>{fmtDate(policy.published_at)}</dd>
      <dt>Change note</dt>
      <dd>{policy.change_note || '—'}</dd>
      <dt>Created by</dt>
      <dd>{policy.created_by || '—'}</dd>
      <dt>Updated at</dt>
      <dd>{fmtDate(policy.updated_at)}</dd>
    </dl>
  );
}

function GlobalFlags({ flags }: { flags: Array<{ key: string; value: boolean }> }) {
  if (flags.length === 0) return null;
  return (
    <div style={{ marginTop: '12px' }}>
      <p className="section-label">Global Flags</p>
      <ul className="flag-list">
        {flags.map((flag) => (
          <li key={flag.key} className="flag-row">
            <span>{flag.value ? '✓' : '✗'}</span>
            <span className={flag.value ? 'ok' : 'err'}>●</span>
            <span className="flag-key">{flag.key}</span>
            <Tag variant={flag.value ? 'ok' : 'err'}>{flag.value ? 'yes' : 'no'}</Tag>
          </li>
        ))}
      </ul>
    </div>
  );
}

function SubsystemsTable({
  terminal,
  mcp,
}: {
  terminal?: { allow_create_actions?: boolean; allow_publish_actions?: boolean } | null;
  mcp?: { allow_mutation_tools?: boolean } | null;
}) {
  const rows: Array<{ label: string; value: boolean }> = [
    ...Object.entries(terminal ?? {}).map(([k, v]) => ({
      label: `terminal.${k.replaceAll('_', ' ')}`,
      value: Boolean(v),
    })),
    ...Object.entries(mcp ?? {}).map(([k, v]) => ({
      label: `mcp.${k.replaceAll('_', ' ')}`,
      value: Boolean(v),
    })),
  ];
  if (rows.length === 0) return null;
  return (
    <div style={{ marginTop: '12px' }}>
      <p className="section-label">Subsystems</p>
      <table className="action-table">
        <tbody>
          {rows.map((r) => (
            <tr key={r.label}>
              <td className="dim" style={{ fontSize: '11px' }}>
                {r.label}
              </td>
              <td>
                <Tag variant={r.value ? 'ok' : 'err'}>{r.value ? 'yes' : 'no'}</Tag>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ActionGroups({
  groups,
}: {
  groups: Record<string, Array<{ action: string; rule: PolicyActionRule }>>;
}) {
  if (Object.keys(groups).length === 0) return null;
  return (
    <div style={{ marginTop: '12px' }}>
      <p className="section-label">Action Rules</p>
      <div className="action-groups">
        {Object.entries(groups).map(([resource, rules]) => (
          <div key={resource} className="action-group">
            <div className="action-group-header">{resource}</div>
            <table className="action-table">
              <tbody>
                {rules.map(({ action, rule }) => (
                  <tr key={action}>
                    <td className="action-name">{action}</td>
                    <td>
                      <Tag variant={rule.enabled ? 'ok' : 'default'}>
                        {rule.enabled ? 'on' : 'off'}
                      </Tag>
                    </td>
                    <td>
                      {rule.requires_approval ? (
                        <Tag variant="purple">approval</Tag>
                      ) : (
                        <span className="dim" style={{ fontSize: '10px' }}>
                          auto
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ))}
      </div>
    </div>
  );
}

function EventTypeTag({ type: eventType }: { type: string }) {
  const variant =
    eventType === 'published'
      ? 'ok'
      : eventType === 'rollback'
        ? 'warn'
        : eventType === 'seeded'
          ? 'accent'
          : 'default';
  return <Tag variant={variant}>{eventType}</Tag>;
}

function EventsFeed({ events }: { events: PolicyEvent[] }) {
  if (events.length === 0) {
    return (
      <p className="muted" style={{ fontSize: '12px' }}>
        No events recorded yet.
      </p>
    );
  }
  return (
    <ul style={{ listStyle: 'none', margin: 0, padding: 0, fontSize: '12px' }}>
      {events.map((ev) => (
        <li key={ev.id} className="policy-event-row">
          <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
            <EventTypeTag type={ev.event_type} />
            <span className="dim" style={{ fontSize: '10px' }}>
              v{ev.version}
            </span>
          </div>
          <span className="policy-event-meta">
            {ev.actor_subject} · {fmtDate(ev.created_at)}
          </span>
        </li>
      ))}
    </ul>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

// Named export for direct use in tests (avoids router context requirement)
export function GovernancePage() {
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const [showRaw, setShowRaw] = useState(false);

  // ── Policy query ───────────────────────────────────────────────────────────
  const policyQuery = useQuery({
    queryKey: queryKeys.governance.policy(),
    queryFn: () => unwrap(apiClient.GET('/api/governance/policy/active')),
    refetchInterval: 30_000,
  });

  // ── Events query ───────────────────────────────────────────────────────────
  const eventsQuery = useQuery({
    queryKey: queryKeys.governance.events(),
    queryFn: () => unwrap(apiClient.GET('/api/governance/policy/events')),
    refetchInterval: 30_000,
  });

  // ── Reload mutation ────────────────────────────────────────────────────────
  const reloadMutation = useMutation({
    mutationFn: () =>
      apiClient.POST('/api/governance/policy/reload').then(({ data, error }) => {
        if (error) throw new Error(String(error));
        return data;
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.governance.all });
      showToast('Policy reloaded');
    },
    onError: (err: Error) => showToast(err.message),
  });

  // ── Derived ────────────────────────────────────────────────────────────────
  const policy = policyQuery.data;

  const globalFlags: Array<{ key: string; value: boolean }> = policy?.policy?.global
    ? Object.entries(policy.policy.global).map(([k, v]) => ({
        key: k.replaceAll('_', ' '),
        value: Boolean(v),
      }))
    : [];

  const actionGroups: Record<string, Array<{ action: string; rule: PolicyActionRule }>> = {};
  for (const [key, rule] of Object.entries(policy?.policy?.actions ?? {})) {
    const dot = key.indexOf('.');
    const resource = dot >= 0 ? key.slice(0, dot) : key;
    const action = dot >= 0 ? key.slice(dot + 1) : key;
    if (!actionGroups[resource]) actionGroups[resource] = [];
    actionGroups[resource].push({ action, rule });
  }

  const events: PolicyEvent[] = eventsQuery.data ?? [];

  // ── Render ─────────────────────────────────────────────────────────────────

  return (
    <div className="gov-page">
      {/* Top bar */}
      <div className="gov-bar">
        <span className="gov-title">Governance</span>
        <span className="muted" style={{ fontSize: '11px' }}>
          Policy configuration and audit log
        </span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: '6px' }}>
          <button
            type="button"
            className="ghost"
            onClick={() => queryClient.invalidateQueries({ queryKey: queryKeys.governance.all })}
          >
            Refresh
          </button>
          <button
            type="button"
            className="ghost"
            onClick={() => reloadMutation.mutate()}
            disabled={reloadMutation.isPending}
            data-testid="reload-btn"
          >
            {reloadMutation.isPending ? '⟳ Reloading…' : 'Reload Policy'}
          </button>
        </div>
      </div>

      {/* Loading */}
      {policyQuery.isLoading && (
        <div style={{ padding: '12px' }}>
          <p className="muted" data-testid="loading-state">
            ⟳ Loading policy…
          </p>
        </div>
      )}

      {/* Error */}
      {policyQuery.isError && (
        <div style={{ padding: '12px' }}>
          <p className="error" data-testid="error-state">
            ✗ Failed to load policy — {(policyQuery.error as Error)?.message ?? 'unknown error'}
          </p>
        </div>
      )}

      {/* Data */}
      {policy && (
        <div className="pane-row" style={{ flex: 1, minHeight: 0 }}>
          {/* Left: policy details */}
          <div className="pane" style={{ flex: 1, minWidth: 0 }}>
            <div className="pane-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                <span className="pane-title">Active Policy</span>
                <Tag variant={stateVariant(policy.state)}>{policy.state}</Tag>
                <span className="dim" style={{ fontSize: '11px' }}>
                  v{policy.version}
                </span>
              </div>
              <button
                type="button"
                className="ghost"
                style={{ fontSize: '11px', padding: '2px 6px' }}
                onClick={() => setShowRaw((v) => !v)}
              >
                {showRaw ? 'Hide raw' : 'Show raw'}
              </button>
            </div>

            <div className="pane-body" style={{ padding: '10px' }}>
              <PolicyMeta policy={policy} />

              {showRaw ? (
                <pre
                  data-testid="raw-policy"
                  style={{
                    marginTop: '10px',
                    maxHeight: '320px',
                    overflowY: 'auto',
                    fontSize: '11px',
                  }}
                >
                  {JSON.stringify(policy.policy, null, 2)}
                </pre>
              ) : (
                <>
                  <GlobalFlags flags={globalFlags} />
                  <SubsystemsTable terminal={policy.policy?.terminal} mcp={policy.policy?.mcp} />
                  <ActionGroups groups={actionGroups} />
                </>
              )}
            </div>
          </div>

          {/* Right: events feed */}
          <div
            className="pane"
            style={{ width: '340px', flexShrink: 0 }}
            data-testid="events-panel"
          >
            <div className="pane-header">
              <span className="pane-title">Policy Events</span>
              {eventsQuery.isFetching && (
                <span className="muted" style={{ fontSize: '10px' }}>
                  ⟳
                </span>
              )}
            </div>
            <div className="pane-body" style={{ padding: '10px' }}>
              {eventsQuery.isError ? (
                <p className="error" style={{ fontSize: '12px' }}>
                  Failed to load events
                </p>
              ) : (
                <EventsFeed events={events} />
              )}
            </div>
          </div>
        </div>
      )}

      {/* Empty state */}
      {!policyQuery.isLoading && !policyQuery.isError && !policy && (
        <div className="empty-state" data-testid="empty-state">
          <div className="empty-icon">⊙</div>
          <div className="empty-title">No policies configured</div>
          <div className="empty-body">
            Governance policies control what agents can do. No active policy has been published yet.
          </div>
        </div>
      )}
      {/* ── Governance design-system styles ── */}
      <style>{`
        /* .gov-page, .gov-bar, .gov-title — now in app.css */
        .gov-page {
          display: flex;
          flex-direction: column;
          height: 100%;
          overflow: hidden;
        }

        /* ── Policy meta key-value list ──────────────────────────────────── */
        .policy-meta {
          display: grid;
          grid-template-columns: max-content 1fr;
          gap: 3px 12px;
          margin: 0;
          font-size: 12px;
        }
        .policy-meta dt {
          color: var(--dim);
          font-size: 11px;
        }
        .policy-meta dd {
          margin: 0;
          color: var(--text-2);
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
        }

        /* ── Flag list ───────────────────────────────────────────────────── */
        .flag-list {
          list-style: none;
          margin: 0;
          padding: 0;
        }
        .flag-row {
          display: flex;
          align-items: center;
          gap: 6px;
          padding: 3px 0;
          font-size: 12px;
          border-bottom: 1px solid var(--border-subtle);
        }
        .flag-key {
          flex: 1;
          color: var(--text-2);
        }

        /* ── Action rules table ──────────────────────────────────────────── */
        .action-groups {
          display: flex;
          flex-direction: column;
          gap: 8px;
        }
        .action-group {
          border: 1px solid var(--border-subtle);
        }
        .action-group-header {
          padding: 4px 10px;
          font-size: 10px;
          font-weight: 600;
          text-transform: uppercase;
          letter-spacing: 0.08em;
          color: var(--dim);
          background: var(--surface);
          border-bottom: 1px solid var(--border-subtle);
        }
        .action-table {
          width: 100%;
          font-size: 12px;
        }
        .action-table td {
          padding: 3px 10px;
          border-bottom: 1px solid var(--border-subtle);
        }
        .action-table tr:last-child td { border-bottom: none; }
        .action-name { color: var(--text-2); }

        /* ── Policy events feed ──────────────────────────────────────────── */
        .policy-event-row {
          padding: 6px 0;
          border-bottom: 1px solid var(--border-subtle);
          display: flex;
          flex-direction: column;
          gap: 2px;
        }
        .policy-event-meta {
          color: var(--muted);
          font-size: 10px;
        }

        /* ── Empty state ─────────────────────────────────────────────────── */
        .empty-state {
          flex: 1;
          display: flex;
          flex-direction: column;
          align-items: center;
          justify-content: center;
          gap: 8px;
          padding: 40px 20px;
        }
        .empty-icon {
          font-size: 32px;
          color: var(--border-2);
          line-height: 1;
        }
        .empty-title {
          font-size: 14px;
          font-weight: 600;
          color: var(--text-2);
        }
        .empty-body {
          font-size: 12px;
          color: var(--muted);
          text-align: center;
          max-width: 340px;
          line-height: 1.6;
        }
      `}</style>
    </div>
  );
}
