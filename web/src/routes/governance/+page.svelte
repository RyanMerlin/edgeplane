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

  function eventTypeClass(t: string) {
    if (t === 'published') return 'status-done';
    if (t === 'created' || t === 'updated') return 'status-progress';
    if (t === 'rolled_back' || t === 'deleted') return 'status-blocked';
    return 'status-proposed';
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

<div class="glass-panel">
  <div class="grid" style="align-items:center;">
    <div>
      <h3 style="margin:0 0 0.2rem">Governance</h3>
      <p class="muted" style="margin:0; font-size:0.85rem;">Policy configuration and audit log</p>
    </div>
    <div style="display:flex; gap:0.5rem; justify-content:flex-end; flex-wrap:wrap;">
      <button class="ghost" onclick={() => queryClient.invalidateQueries({ queryKey: queryKeys.governance.all })}>
        Refresh
      </button>
      <button
        class="ghost"
        onclick={() => reloadMutation.mutate()}
        disabled={reloadMutation.isPending}
      >
        {reloadMutation.isPending ? 'Reloading…' : 'Reload Policy'}
      </button>
    </div>
  </div>

  {#if policyQuery.isLoading}
    <p class="muted" style="margin-top:1rem;">Loading policy…</p>
  {:else if policyQuery.isError}
    <p class="error" style="margin-top:1rem;">Failed to load policy — {(policyQuery.error as Error)?.message ?? 'unknown error'}</p>
  {:else if policy}
    <div class="grid" style="margin-top:1rem; gap:1rem; align-items:start;">

      <!-- Left: policy details -->
      <section class="glass-panel">
        <div style="display:flex; justify-content:space-between; align-items:center; flex-wrap:wrap; gap:0.5rem; margin-bottom:0.75rem;">
          <div style="display:flex; align-items:center; gap:0.6rem; flex-wrap:wrap;">
            <h4 style="margin:0;">Active Policy</h4>
            <span class={`status-badge ${stateClass(policy.state)}`}>{policy.state}</span>
            <span class="status-chip">v{policy.version}</span>
          </div>
          <button class="ghost" style="font-size:0.8rem;" onclick={() => (showRawPolicy = !showRawPolicy)}>
            {showRawPolicy ? 'Hide raw' : 'Show raw'}
          </button>
        </div>

        <dl class="policy-meta">
          <dt>Published by</dt><dd>{policy.published_by || '—'}</dd>
          <dt>Published at</dt><dd>{fmtDate(policy.published_at)}</dd>
          <dt>Change note</dt><dd>{policy.change_note || '—'}</dd>
          <dt>Created by</dt><dd>{policy.created_by || '—'}</dd>
          <dt>Updated at</dt><dd>{fmtDate(policy.updated_at)}</dd>
        </dl>

        {#if showRawPolicy}
          <pre style="margin-top:0.75rem; max-height:320px; overflow-y:auto;">{JSON.stringify(policy.policy, null, 2)}</pre>
        {:else}
          <!-- Global flags -->
          {#if globalFlags.length > 0}
            <div style="margin-top:0.9rem;">
              <p class="section-label">Global Flags</p>
              <ul class="flag-list">
                {#each globalFlags as flag}
                  <li class="flag-row">
                    <span class={`flag-dot ${flag.value ? 'flag-on' : 'flag-off'}`}></span>
                    <span class="flag-key">{flag.key}</span>
                    <span class={`status-badge ${flag.value ? 'status-done' : 'status-blocked'}`}>{flag.value ? 'yes' : 'no'}</span>
                  </li>
                {/each}
              </ul>
            </div>
          {/if}

          <!-- Terminal + MCP subsystems -->
          {#if policy.policy?.terminal || policy.policy?.mcp}
            <div style="margin-top:0.9rem;">
              <p class="section-label">Subsystems</p>
              <div style="display:flex; flex-wrap:wrap; gap:0.4rem;">
                {#each Object.entries(policy.policy?.terminal ?? {}) as [k, v]}
                  <div class="subsys-chip">
                    <span class="muted">terminal.{k.replaceAll('_', ' ')}</span>
                    <span class={`status-badge ${v ? 'status-done' : 'status-blocked'}`}>{v ? 'yes' : 'no'}</span>
                  </div>
                {/each}
                {#each Object.entries(policy.policy?.mcp ?? {}) as [k, v]}
                  <div class="subsys-chip">
                    <span class="muted">mcp.{k.replaceAll('_', ' ')}</span>
                    <span class={`status-badge ${v ? 'status-done' : 'status-blocked'}`}>{v ? 'yes' : 'no'}</span>
                  </div>
                {/each}
              </div>
            </div>
          {/if}

          <!-- Action rules -->
          {#if Object.keys(actionGroups).length > 0}
            <div style="margin-top:0.9rem;">
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
                            <td>
                              <span class={`status-badge ${rule.enabled ? 'status-done' : 'status-blocked'}`}>
                                {rule.enabled ? 'on' : 'off'}
                              </span>
                            </td>
                            <td>
                              {#if rule.requires_approval}
                                <span class="status-badge status-proposed">approval</span>
                              {:else}
                                <span class="muted" style="font-size:0.73rem;">auto</span>
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
      </section>

      <!-- Right: events feed -->
      <section class="glass-panel">
        <div style="display:flex; justify-content:space-between; align-items:center; flex-wrap:wrap; gap:0.5rem; margin-bottom:0.75rem;">
          <h4 style="margin:0;">Policy Events <span class="status-chip">{filteredEvents.length}</span></h4>
          <select bind:value={eventFilter} class="filter-select">
            <option value="">All types</option>
            {#each eventTypes as et}
              <option value={et}>{et}</option>
            {/each}
          </select>
        </div>

        {#if eventsQuery.isLoading}
          <p class="muted">Loading events…</p>
        {:else if filteredEvents.length === 0}
          <p class="muted">No events recorded yet.</p>
        {:else}
          <ul class="event-feed">
            {#each filteredEvents as evt (evt.id)}
              <li class="event-item">
                <div class="event-item-header">
                  <span class={`status-badge ${eventTypeClass(evt.event_type)}`}>{evt.event_type}</span>
                  <span class="muted" style="font-size:0.78rem;">{evt.actor_subject}</span>
                  <span class="muted" style="font-size:0.75rem; margin-left:auto;">{fmtRelative(evt.created_at)}</span>
                </div>
                <div style="display:flex; gap:0.4rem; margin-top:0.3rem; align-items:center; flex-wrap:wrap;">
                  <span class="status-chip">v{evt.version}</span>
                  {#if Object.keys(evt.detail ?? {}).length > 0}
                    <details>
                      <summary>detail</summary>
                      <pre style="font-size:0.75rem; margin-top:0.25rem;">{JSON.stringify(evt.detail, null, 2)}</pre>
                    </details>
                  {/if}
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

    </div>
  {:else}
    <p class="muted" style="margin-top:1rem;">No active policy found.</p>
  {/if}
</div>

<style>
  .section-label {
    margin: 0 0 0.4rem;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--muted);
  }

  .policy-meta {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.25rem 0.75rem;
    font-size: 0.84rem;
    margin: 0;
  }
  .policy-meta dt { color: var(--muted); font-size: 0.78rem; padding-top: 0.05rem; }
  .policy-meta dd { margin: 0; }

  .flag-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.3rem; }
  .flag-row { display: flex; align-items: center; gap: 0.5rem; font-size: 0.84rem; }
  .flag-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; }
  .flag-on { background: #34d399; }
  .flag-off { background: #fb7185; }
  .flag-key { flex: 1; }

  .subsys-chip {
    display: flex; align-items: center; gap: 0.4rem;
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--panel-border);
    border-radius: 0.5rem;
    padding: 0.28rem 0.5rem;
    font-size: 0.79rem;
  }

  .action-groups {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(155px, 1fr));
    gap: 0.5rem;
  }
  .action-group { border: 1px solid var(--panel-border); border-radius: 0.55rem; overflow: hidden; }
  .action-group-header {
    background: rgba(255,255,255,0.05);
    padding: 0.25rem 0.55rem;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
  }
  .action-table { width: 100%; border-collapse: collapse; font-size: 0.79rem; }
  .action-table tr + tr { border-top: 1px solid var(--panel-border); }
  .action-table td { padding: 0.22rem 0.45rem; vertical-align: middle; }
  .action-name { color: var(--text); }

  .filter-select {
    background: rgba(255,255,255,0.05);
    border: 1px solid var(--panel-border);
    border-radius: 0.4rem;
    padding: 0.28rem 0.55rem;
    color: var(--text);
    font-size: 0.8rem;
  }

  .event-feed {
    list-style: none; margin: 0; padding: 0;
    display: flex; flex-direction: column; gap: 0.45rem;
    max-height: 540px; overflow-y: auto;
  }
  .event-item {
    background: rgba(255,255,255,0.03);
    border: 1px solid var(--panel-border);
    border-radius: 0.55rem;
    padding: 0.55rem 0.7rem;
  }
  .event-item-header { display: flex; align-items: center; gap: 0.45rem; flex-wrap: wrap; }
</style>
