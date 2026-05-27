<script lang="ts">
  import { createQuery, createMutation, useQueryClient } from '@tanstack/svelte-query';
  import { useAuthState } from '$lib/stores/auth-state.svelte';
  import { showToast } from '$lib/stores/toast';
  import { queryKeys } from '$lib/queryKeys';
  import {
    fetchPolicy, fetchGovernanceEvents, reloadPolicy,
    type PolicyRecord, type PolicyEvent, type PolicyActionRule
  } from '$lib/api';

  const queryClient = useQueryClient();
  const auth = useAuthState();

  let eventFilter = $state<string>('');
  let showRawPolicy = $state(false);

  // ── Queries ────────────────────────────────────────────────────────────────

  const policyQuery = createQuery(() => ({
    queryKey: queryKeys.governance.policy(),
    queryFn: () => fetchPolicy(auth.currentToken || undefined),
    enabled: auth.isLoggedIn
  }));

  const eventsQuery = createQuery(() => ({
    queryKey: queryKeys.governance.events(),
    queryFn: () => fetchGovernanceEvents(100, auth.currentToken || undefined),
    enabled: auth.isLoggedIn,
    refetchInterval: 30_000
  }));

  const reloadMutation = createMutation(() => ({
    mutationFn: () => reloadPolicy(auth.currentToken || undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.governance.all });
      showToast('Policy reloaded');
    },
    onError: (err: Error) => showToast(err.message)
  }));

  // ── Derived ────────────────────────────────────────────────────────────────

  let policy = $derived(policyQuery.data as PolicyRecord | undefined);
  let events = $derived((eventsQuery.data ?? []) as PolicyEvent[]);

  let filteredEvents = $derived(
    eventFilter ? events.filter(e => e.event_type === eventFilter) : events
  );

  let eventTypes = $derived([...new Set(events.map(e => e.event_type))].sort());

  let actionGroups = $derived.by(() => {
    const actions = policy?.policy?.actions ?? {};
    const groups: Record<string, Array<{ action: string; rule: PolicyActionRule }>> = {};
    for (const [key, rule] of Object.entries(actions)) {
      const dot = key.indexOf('.');
      const resource = dot >= 0 ? key.slice(0, dot) : key;
      const action = dot >= 0 ? key.slice(dot + 1) : key;
      if (!groups[resource]) groups[resource] = [];
      groups[resource].push({ action, rule: rule as PolicyActionRule });
    }
    return groups;
  });

  let globalFlags = $derived.by(() => {
    const g = policy?.policy?.global;
    if (!g) return [];
    return Object.entries(g).map(([k, v]) => ({
      key: k.replaceAll('_', ' '),
      value: v as boolean
    }));
  });

  // ── Helpers ────────────────────────────────────────────────────────────────

  function stateClass(state?: string) {
    if (state === 'active') return 'status-done';
    if (state === 'draft') return 'status-proposed';
    if (state === 'archived') return 'status-blocked';
    return '';
  }

  function stateTagClass(state?: string) {
    if (state === 'active') return 'ok';
    if (state === 'draft') return 'warn';
    if (state === 'archived') return 'dim';
    return 'dim';
  }

  function eventTypeClass(t: string) {
    if (t === 'published') return 'status-done';
    if (t === 'created' || t === 'updated') return 'status-progress';
    if (t === 'rolled_back' || t === 'deleted') return 'status-blocked';
    return 'status-proposed';
  }

  function evtTagClass(t: string) {
    if (t === 'published') return 'ok';
    if (t === 'created' || t === 'updated') return 'accent';
    if (t === 'rolled_back' || t === 'deleted') return 'err';
    return 'dim';
  }

  function fmtDate(s: string | null | undefined) {
    if (!s) return '—';
    return new Date(s).toLocaleString();
  }

  function fmtRelative(s: string) {
    const diff = Date.now() - new Date(s).getTime();
    const m = Math.floor(diff / 60_000);
    if (m < 1) return 'just now';
    if (m < 60) return `${m}m ago`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ago`;
    return `${Math.floor(h / 24)}d ago`;
  }
</script>

<div class="gov-page">

  <!-- topbar for governance -->
  <div class="gov-bar">
    <span class="gov-title">Governance</span>
    <span class="muted" style="font-size:11px;">Policy configuration and audit log</span>
    <div style="margin-left:auto; display:flex; gap:6px;">
      <button class="ghost" onclick={() => queryClient.invalidateQueries({ queryKey: queryKeys.governance.all })}>Refresh</button>
      <button class="ghost" onclick={() => reloadMutation.mutate()} disabled={reloadMutation.isPending}>
        {reloadMutation.isPending ? '⟳ Reloading…' : 'Reload Policy'}
      </button>
    </div>
  </div>

  {#if policyQuery.isLoading}
    <div style="padding:12px;"><p class="muted">⟳ Loading policy…</p></div>
  {:else if policyQuery.isError}
    <div style="padding:12px;"><p class="error">✗ Failed to load policy — {(policyQuery.error as Error)?.message ?? 'unknown error'}</p></div>
  {:else if policy}
    <div class="pane-row" style="flex:1; min-height:0;">

      <!-- left: policy details -->
      <div class="pane" style="flex:1; min-width:0;">
        <div class="pane-header">
          <div style="display:flex; align-items:center; gap:8px;">
            <span class="pane-title">Active Policy</span>
            <span class="tag {stateTagClass(policy.state)}">{policy.state}</span>
            <span class="dim" style="font-size:11px;">v{policy.version}</span>
          </div>
          <button class="ghost" style="font-size:11px; padding:2px 6px;" onclick={() => (showRawPolicy = !showRawPolicy)}>
            {showRawPolicy ? 'Hide raw' : 'Show raw'}
          </button>
        </div>
        <div class="pane-body" style="padding:10px;">

          <dl class="policy-meta">
            <dt>Published by</dt><dd>{policy.published_by || '—'}</dd>
            <dt>Published at</dt><dd>{fmtDate(policy.published_at)}</dd>
            <dt>Change note</dt><dd>{policy.change_note || '—'}</dd>
            <dt>Created by</dt><dd>{policy.created_by || '—'}</dd>
            <dt>Updated at</dt><dd>{fmtDate(policy.updated_at)}</dd>
          </dl>

          {#if showRawPolicy}
            <pre style="margin-top:10px; max-height:320px; overflow-y:auto; font-size:11px;">{JSON.stringify(policy.policy, null, 2)}</pre>
          {:else}

            {#if globalFlags.length > 0}
              <div style="margin-top:12px;">
                <p class="section-label">Global Flags</p>
                <ul class="flag-list">
                  {#each globalFlags as flag}
                    <li class="flag-row">
                      <span>{flag.value ? '✓' : '✗'}</span>
                      <span class={flag.value ? 'ok' : 'err'}>{flag.value ? '●' : '●'}</span>
                      <span class="flag-key">{flag.key}</span>
                      <span class="tag {flag.value ? 'ok' : 'err'}">{flag.value ? 'yes' : 'no'}</span>
                    </li>
                  {/each}
                </ul>
              </div>
            {/if}

            {#if policy.policy?.terminal || policy.policy?.mcp}
              <div style="margin-top:12px;">
                <p class="section-label">Subsystems</p>
                <table class="action-table">
                  <tbody>
                    {#each Object.entries(policy.policy?.terminal ?? {}) as [k, v]}
                      <tr>
                        <td class="dim" style="font-size:11px;">terminal.{k.replaceAll('_', ' ')}</td>
                        <td><span class="tag {v ? 'ok' : 'err'}">{v ? 'yes' : 'no'}</span></td>
                      </tr>
                    {/each}
                    {#each Object.entries(policy.policy?.mcp ?? {}) as [k, v]}
                      <tr>
                        <td class="dim" style="font-size:11px;">mcp.{k.replaceAll('_', ' ')}</td>
                        <td><span class="tag {v ? 'ok' : 'err'}">{v ? 'yes' : 'no'}</span></td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {/if}

            {#if Object.keys(actionGroups).length > 0}
              <div style="margin-top:12px;">
                <p class="section-label">Action Rules</p>
                <div class="action-groups">
                  {#each Object.entries(actionGroups) as [resource, rules]}
                    <div class="action-group">
                      <div class="action-group-header">{resource}</div>
                      <table class="action-table">
                        <tbody>
                          {#each rules as { action, rule }}
                            <tr>
                              <td class="action-name">{action}</td>
                              <td><span class="tag {rule.enabled ? 'ok' : 'dim'}">{rule.enabled ? 'on' : 'off'}</span></td>
                              <td>
                                {#if rule.requires_approval}
                                  <span class="tag purple">approval</span>
                                {:else}
                                  <span class="dim" style="font-size:10px;">auto</span>
                                {/if}
                              </td>
                            </tr>
                          {/each}
                        </tbody>
                      </table>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}
          {/if}

        </div>
      </div>

      <!-- right: events feed -->
      <div class="pane" style="width:340px; flex-shrink:0;">
        <div class="pane-header">
          <span class="pane-title">Policy Events</span>
          <div style="display:flex; align-items:center; gap:6px;">
            <span class="dim" style="font-size:11px;">{filteredEvents.length}</span>
            <select bind:value={eventFilter} style="font-size:11px; padding:2px 5px;">
              <option value="">All types</option>
              {#each eventTypes as et}
                <option value={et}>{et}</option>
              {/each}
            </select>
          </div>
        </div>
        <div class="pane-body">
          {#if eventsQuery.isLoading}
            <div style="padding:10px;"><p class="muted">⟳ Loading…</p></div>
          {:else if filteredEvents.length === 0}
            <div style="padding:10px;"><p class="muted">No events recorded yet.</p></div>
          {:else}
            {#each filteredEvents as evt (evt.id)}
              <div class="event-row">
                <div style="display:flex; align-items:center; gap:6px; flex-wrap:wrap;">
                  <span class="tag {evtTagClass(evt.event_type)}">{evt.event_type}</span>
                  <span class="muted" style="font-size:11px;">{evt.actor_subject}</span>
                  <span class="dim" style="font-size:10px; margin-left:auto;">{fmtRelative(evt.created_at)}</span>
                </div>
                <div style="margin-top:3px; display:flex; gap:6px; align-items:center; font-size:11px;">
                  <span class="dim">v{evt.version}</span>
                  {#if Object.keys(evt.detail ?? {}).length > 0}
                    <details>
                      <summary>detail</summary>
                      <pre style="font-size:11px; margin-top:3px;">{JSON.stringify(evt.detail, null, 2)}</pre>
                    </details>
                  {/if}
                </div>
              </div>
            {/each}
          {/if}
        </div>
      </div>

    </div>
  {:else}
    <div style="padding:12px;"><p class="muted">No active policy found.</p></div>
  {/if}

</div>

<style>
  .gov-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .gov-bar {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .gov-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }

  .policy-meta {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 3px 10px;
    font-size: 12px;
    margin: 0;
  }
  .policy-meta dt { color: var(--muted); font-size: 11px; }
  .policy-meta dd { margin: 0; }

  .flag-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .flag-row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    padding: 3px 0;
    border-bottom: 1px solid var(--border);
  }
  .flag-key { flex: 1; }

  .action-groups {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 6px;
  }
  .action-group {
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .action-group-header {
    background: var(--surface);
    padding: 3px 7px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
    border-bottom: 1px solid var(--border);
  }
  .action-table { width: 100%; border-collapse: collapse; font-size: 11px; }
  .action-table tr + tr { border-top: 1px solid var(--border); }
  .action-table td { padding: 3px 7px; vertical-align: middle; }
  .action-name { color: var(--text); }

  .event-row {
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .event-row:hover { background: var(--surface-2); }
</style>
