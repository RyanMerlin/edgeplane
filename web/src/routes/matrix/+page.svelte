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

  function statusTagClass(s?: string) {
    const v = String(s ?? '').toLowerCase();
    if (v === 'done' || v === 'completed' || v === 'ok') return 'ok';
    if (v === 'in_progress' || v === 'running') return 'accent';
    if (v === 'blocked' || v === 'failed' || v === 'error') return 'err';
    return 'dim';
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

  function typeTagClass(t: string) {
    const v = t.toLowerCase();
    if (v.includes('domain')) return 'err';
    if (v.includes('mission')) return 'purple';
    if (v.includes('task')) return 'accent';
    if (v.includes('agent')) return 'ok';
    if (v.includes('error') || v.includes('fail')) return 'err';
    return 'dim';
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

<div class="matrix-page">

  <!-- filter bar -->
  <div class="matrix-bar">
    <span class="matrix-title">Feed</span>
    <span class={$matrixStatus.connected ? 'ok' : 'dim'}>{$matrixStatus.connected ? '●' : '○'}</span>
    <span class="muted" style="font-size:11px;">{$matrixStatus.connected ? 'live' : 'offline'}</span>
    <span class="dim" style="font-size:11px; margin-left:4px;">{$matrixEvents.length} events · {$recentRate}/min</span>
    {#if $matrixStatus.rateLimit}
      <span class="dim" style="font-size:11px;">· rl {$matrixStatus.rateLimit.remaining}/{$matrixStatus.rateLimit.limit}</span>
    {/if}
    <div style="margin-left:auto; display:flex; gap:6px; align-items:center;">
      <select bind:value={filterType} style="font-size:11px; padding:2px 5px;">
        <option value="">All types</option>
        {#each $knownTypes as t}
          <option value={t}>{t}</option>
        {/each}
      </select>
      <button class="ghost" onclick={clearEvents}>Clear</button>
    </div>
  </div>

  {#if $matrixStatus.lastError && !$matrixStatus.connected}
    <div style="padding:6px 12px; border-bottom:1px solid var(--border);">
      <p class="error" style="margin:0; font-size:11px;">✗ {$matrixStatus.lastError}</p>
    </div>
  {/if}

  <!-- event list -->
  <div class="matrix-list">
    {#if $visibleEvents.length === 0}
      <div class="matrix-empty">
        <p class="muted">{filterType ? `No "${filterType}" events yet.` : 'Waiting for events…'}</p>
      </div>
    {:else}
      {#each $visibleEvents as evt, i (evt.receivedAt + '-' + i)}
        {@const label = evt.type ?? evt.event ?? 'matrix'}
        {@const summary = summaryOf(evt.payload)}
        <div class="event-row">
          <div class="event-time">{fmtTime(evt.receivedAt)}</div>
          <div class="event-label">
            <span class="tag {typeTagClass(label)}">{label}</span>
          </div>
          <div class="event-meta">
            {#if evt.status}
              <span class="tag {statusTagClass(evt.status)}">{evt.status}</span>
            {/if}
            {#if evt.mission_id}
              <span class="dim" style="font-size:10px;">m:{evt.mission_id.slice(0,8)}</span>
            {/if}
            {#if evt.agent_id}
              <span class="dim" style="font-size:10px;">a:{evt.agent_id.slice(0,8)}</span>
            {/if}
          </div>
          <div class="event-summary-col">
            {#if summary}
              <span class="muted" style="font-size:11px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;">{summary}</span>
            {/if}
          </div>
          <div class="event-detail-col">
            {#if Object.keys(evt.payload ?? {}).length > 0}
              <details>
                <summary>payload</summary>
                <pre style="font-size:11px; margin-top:3px;">{JSON.stringify(evt.payload, null, 2)}</pre>
              </details>
            {/if}
          </div>
        </div>
      {/each}

      {#if $matrixEvents.length > maxVisible}
        <div style="padding:8px; text-align:center; border-top:1px solid var(--border);">
          <button class="ghost" onclick={() => (maxVisible += 50)}>
            Show more ({$matrixEvents.length - maxVisible} remaining)
          </button>
        </div>
      {/if}
    {/if}
  </div>

</div>

<style>
  .matrix-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .matrix-bar {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .matrix-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    margin-right: 2px;
  }

  .matrix-list {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }

  .matrix-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    min-height: 80px;
  }

  .event-row {
    display: grid;
    grid-template-columns: 72px 140px 1fr 2fr auto;
    gap: 8px;
    align-items: center;
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    animation: fade-in 180ms ease-out;
  }
  .event-row:hover { background: var(--surface-2); }

  .event-time {
    font-size: 10px;
    color: var(--dim);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .event-label { display: flex; align-items: center; gap: 4px; }
  .event-meta  { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
  .event-summary-col { overflow: hidden; display: flex; align-items: center; }
  .event-detail-col  { display: flex; align-items: center; }
</style>
