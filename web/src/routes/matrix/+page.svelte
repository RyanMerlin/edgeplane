<script lang="ts">
  import { derived } from 'svelte/store';
  import { matrixEvents, matrixStatus } from '$lib/telemetry';

  let filterType = $state('');
  let maxVisible = $state(50);

  // All unique event types seen so far
  const knownTypes = derived(matrixEvents, $evts =>
    [...new Set($evts.map(e => e.type ?? e.event ?? '').filter(Boolean))].sort()
  );

  // Rate: events received in the last 60 s
  const recentRate = derived(matrixEvents, $evts => {
    const cutoff = Date.now() - 60_000;
    return $evts.filter(e => e.receivedAt > cutoff).length;
  });

  // Filtered + capped view
  const visibleEvents = derived(matrixEvents, $evts => {
    const filtered = filterType
      ? $evts.filter(e => (e.type ?? e.event ?? '') === filterType)
      : $evts;
    return filtered.slice(0, maxVisible);
  });

  function clearEvents() {
    matrixEvents.set([]);
  }

  function statusClass(s?: string) {
    const v = String(s ?? '').toLowerCase();
    if (v === 'done' || v === 'completed' || v === 'ok') return 'status-done';
    if (v === 'in_progress' || v === 'running') return 'status-progress';
    if (v === 'blocked' || v === 'failed' || v === 'error') return 'status-blocked';
    return 'status-proposed';
  }

  function typeColor(t: string) {
    const v = t.toLowerCase();
    if (v.includes('domain')) return 'type-domain';
    if (v.includes('mission')) return 'type-mission';
    if (v.includes('task')) return 'type-task';
    if (v.includes('agent')) return 'type-agent';
    if (v.includes('error') || v.includes('fail')) return 'type-error';
    return '';
  }

  function summaryOf(payload: unknown): string {
    if (!payload || typeof payload !== 'object') return String(payload ?? '');
    const p = payload as Record<string, unknown>;
    return String(p.summary ?? p.message ?? p.title ?? p.name ?? '').slice(0, 120) || JSON.stringify(payload).slice(0, 120);
  }

  function fmtTime(ts: number) {
    return new Date(ts).toLocaleTimeString();
  }
</script>

<div class="glass-panel">
  <!-- Header / stats bar -->
  <div class="matrix-header">
    <div style="display:flex; align-items:center; gap:0.75rem; flex-wrap:wrap;">
      <h3 style="margin:0;">Matrix</h3>
      <span class={`conn-badge ${$matrixStatus.connected ? 'conn-live' : 'conn-off'}`}>
        {$matrixStatus.connected ? '● live' : '○ offline'}
      </span>
    </div>
    <div class="stats-row">
      <span class="status-chip">{$matrixEvents.length} events</span>
      <span class="status-chip">{$recentRate}/min</span>
      {#if $matrixStatus.rateLimit}
        <span class="status-chip">rl {$matrixStatus.rateLimit.remaining}/{$matrixStatus.rateLimit.limit}</span>
      {/if}
    </div>
    <div style="display:flex; gap:0.45rem; align-items:center; flex-wrap:wrap;">
      <select bind:value={filterType} class="filter-select">
        <option value="">All types</option>
        {#each $knownTypes as t}
          <option value={t}>{t}</option>
        {/each}
      </select>
      <button class="ghost" onclick={clearEvents}>Clear</button>
    </div>
  </div>

  {#if $matrixStatus.lastError && !$matrixStatus.connected}
    <p class="error" style="margin:0.5rem 0 0;">{$matrixStatus.lastError}</p>
  {/if}

  <!-- Event timeline -->
  {#if $visibleEvents.length === 0}
    <div class="empty-state">
      <p class="muted">{filterType ? `No "${filterType}" events yet.` : 'Waiting for events…'}</p>
    </div>
  {:else}
    <div class="matrix-timeline">
      {#each $visibleEvents as evt, i (evt.receivedAt + '-' + i)}
        {@const label = evt.type ?? evt.event ?? 'matrix'}
        {@const summary = summaryOf(evt.payload)}
        <div class="matrix-event">
          <div class="event-time">{fmtTime(evt.receivedAt)}</div>
          <div class="event-body">
            <div class="event-top">
              <span class={`type-badge ${typeColor(label)}`}>{label}</span>
              {#if evt.status}
                <span class={`status-badge ${statusClass(evt.status)}`}>{evt.status}</span>
              {/if}
              {#if evt.mission_id}
                <span class="status-chip" style="font-size:0.73rem;">mission:{evt.mission_id.slice(0,8)}</span>
              {/if}
              {#if evt.agent_id}
                <span class="status-chip" style="font-size:0.73rem;">agent:{evt.agent_id.slice(0,8)}</span>
              {/if}
            </div>
            {#if summary}
              <p class="event-summary muted">{summary}</p>
            {/if}
            {#if Object.keys(evt.payload ?? {}).length > 0}
              <details>
                <summary>payload</summary>
                <pre style="font-size:0.75rem; margin-top:0.25rem;">{JSON.stringify(evt.payload, null, 2)}</pre>
              </details>
            {/if}
          </div>
        </div>
      {/each}
    </div>

    {#if $matrixEvents.length > maxVisible}
      <div style="text-align:center; margin-top:0.75rem;">
        <button class="ghost" onclick={() => (maxVisible += 50)}>
          Show more ({$matrixEvents.length - maxVisible} remaining)
        </button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .matrix-header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem;
    margin-bottom: 0.85rem;
  }
  .stats-row { display: flex; gap: 0.4rem; flex-wrap: wrap; }

  .conn-badge {
    font-size: 0.8rem;
    padding: 0.2rem 0.6rem;
    border-radius: 999px;
    border: 1px solid transparent;
  }
  .conn-live { color: #34d399; border-color: rgba(52,211,153,0.4); background: rgba(52,211,153,0.12); }
  .conn-off  { color: #9aa7c4; border-color: rgba(154,167,196,0.3); background: rgba(154,167,196,0.08); }

  .filter-select {
    background: rgba(255,255,255,0.05);
    border: 1px solid var(--panel-border);
    border-radius: 0.4rem;
    padding: 0.28rem 0.55rem;
    color: var(--text);
    font-size: 0.8rem;
  }

  .empty-state {
    min-height: 120px;
    display: grid;
    place-items: center;
  }

  .matrix-timeline {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    max-height: 620px;
    overflow-y: auto;
  }

  .matrix-event {
    display: flex;
    gap: 0.75rem;
    padding: 0.55rem 0.7rem;
    background: rgba(255,255,255,0.03);
    border: 1px solid var(--panel-border);
    border-radius: 0.6rem;
    animation: fade-in 180ms ease-out;
  }

  .event-time {
    font-size: 0.73rem;
    color: var(--muted);
    white-space: nowrap;
    padding-top: 0.15rem;
    min-width: 5rem;
    font-family: 'IBM Plex Mono', ui-monospace, monospace;
  }

  .event-body { flex: 1; min-width: 0; }
  .event-top { display: flex; align-items: center; gap: 0.4rem; flex-wrap: wrap; }
  .event-summary { margin: 0.25rem 0 0; font-size: 0.82rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .type-badge {
    display: inline-flex;
    align-items: center;
    padding: 0.15rem 0.5rem;
    border-radius: 999px;
    font-size: 0.75rem;
    border: 1px solid rgba(255,255,255,0.15);
    background: rgba(255,255,255,0.06);
    color: var(--text);
  }
  .type-domain  { border-color: rgba(217,74,43,0.4); background: rgba(217,74,43,0.12); color: #f87c60; }
  .type-mission { border-color: rgba(167,139,250,0.4); background: rgba(167,139,250,0.12); color: #c4b5fd; }
  .type-task    { border-color: rgba(56,189,248,0.4); background: rgba(56,189,248,0.12); color: #7dd3fc; }
  .type-agent   { border-color: rgba(52,211,153,0.4); background: rgba(52,211,153,0.12); color: #6ee7b7; }
  .type-error   { border-color: rgba(251,113,133,0.4); background: rgba(251,113,133,0.12); color: #fda4af; }
</style>
