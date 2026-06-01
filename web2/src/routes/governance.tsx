/**
 * Governance screen — Phase 0.9 frontend slice.
 *
 * Data source: GET /api/governance/policy/active (typed via schema.gen.ts)
 * Mutation:    POST /api/governance/policy/reload
 * Cadence:     refetchInterval 30s (matches Svelte page)
 *
 * NOTE: GovernancePolicyResponse.policy is typed as `unknown` in schema.gen.ts
 * because the backend DTO uses a serde_json::Value mirror field. We cast it
 * locally to PolicyDoc below — this works at runtime but loses compile-time
 * safety on the nested policy shape. Backend should tighten the DTO to a
 * concrete struct so utoipa emits a real schema for the `policy` object.
 * See report for details.
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useToastStore } from '@/stores/toast';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';

// ── Types ─────────────────────────────────────────────────────────────────────

type PolicyActionRule = {
  enabled: boolean;
  requires_approval: boolean;
};

type PolicyDoc = {
  global?: Record<string, boolean>;
  actions?: Record<string, PolicyActionRule>;
  terminal?: Record<string, boolean>;
  mcp?: Record<string, boolean>;
};

/** The generated type with the `policy` field cast to our local PolicyDoc. */
type PolicyRecord = Omit<components['schemas']['GovernancePolicyResponse'], 'policy'> & {
  policy: PolicyDoc | null;
};

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
  terminal?: Record<string, boolean>;
  mcp?: Record<string, boolean>;
}) {
  const rows: Array<{ label: string; value: boolean }> = [
    ...Object.entries(terminal ?? {}).map(([k, v]) => ({
      label: `terminal.${k.replaceAll('_', ' ')}`,
      value: v,
    })),
    ...Object.entries(mcp ?? {}).map(([k, v]) => ({
      label: `mcp.${k.replaceAll('_', ' ')}`,
      value: v,
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

// ── Main page ─────────────────────────────────────────────────────────────────

// Named export for direct use in tests (avoids router context requirement)
export function GovernancePage() {
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const [showRaw, setShowRaw] = useState(false);

  // ── Policy query ───────────────────────────────────────────────────────────
  const policyQuery = useQuery({
    queryKey: queryKeys.governance.policy(),
    queryFn: () =>
      unwrap(apiClient.GET('/api/governance/policy/active')).then((d) => d as PolicyRecord),
    refetchInterval: 30_000,
  });

  // ── Reload mutation ────────────────────────────────────────────────────────
  const reloadMutation = useMutation({
    mutationFn: () =>
      // POST /api/governance/policy/reload — not in schema.gen.ts (stub only covers active GET)
      // Call via the typed client's raw fetch with CSRF middleware applied.
      // When the backend adds this path to the OpenAPI spec, update to apiClient.POST(...).
      fetch('/api/governance/policy/reload', {
        method: 'POST',
        credentials: 'include',
        headers: (() => {
          const h = new Headers({ 'Content-Type': 'application/json' });
          // CSRF: read cookie and inject header
          if (typeof document !== 'undefined') {
            const needle = 'ep_csrf_token=';
            for (const part of document.cookie.split(';')) {
              const item = part.trim();
              if (item.startsWith(needle)) {
                h.set('X-CSRF-Token', decodeURIComponent(item.slice(needle.length)));
                break;
              }
            }
          }
          return h;
        })(),
      }).then(async (res) => {
        if (!res.ok) {
          const text = await res.text().catch(() => '');
          throw new Error(text || `Reload failed: ${res.status}`);
        }
        return res.json().catch(() => ({ ok: true }));
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

          {/* Right: events feed placeholder (events endpoint not in schema.gen.ts yet) */}
          <div
            className="pane"
            style={{ width: '340px', flexShrink: 0 }}
            data-testid="events-panel"
          >
            <div className="pane-header">
              <span className="pane-title">Policy Events</span>
            </div>
            <div className="pane-body" style={{ padding: '10px' }}>
              <p className="muted" style={{ fontSize: '12px' }}>
                Events feed — deferred until <code>/api/governance/policy/events</code> is added to
                the OpenAPI spec.
              </p>
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
    </div>
  );
}

// ── CSS (scoped via inline style blocks in app.css — no CSS modules here) ─────
// The Svelte version added scoped styles; we rely on the global app.css classes
// already present (gov-page, gov-bar, pane, pane-header, pane-body, pane-row,
// gov-title, policy-meta, flag-list, flag-row, flag-key, action-groups,
// action-group, action-group-header, action-table, action-name, empty-state,
// empty-icon, empty-title, empty-body). If any are absent they'll be no-ops.
