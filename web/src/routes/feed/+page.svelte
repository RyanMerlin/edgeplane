<script lang="ts">
  import { derived } from 'svelte/store';
  import { matrixEvents, matrixStatus } from '$lib/telemetry';

  // ── Filter state ────────────────────────────────────────────────────────────

  let filterText = $state('');
  let activeChip = $state<'all' | 'errors' | 'governance' | 'artifacts' | 'tasks' | 'heartbeat'>('all');
  let selectedIdx = $state<number | null>(null);
  let alertsOnly = $state(false);

  // ── Derived ─────────────────────────────────────────────────────────────────

  const recentRate = derived(matrixEvents, $evts => {
    const cutoff = Date.now() - 60_000;
    return $evts.filter(e => e.receivedAt > cutoff).length;
  });

  const filteredEvents = derived(matrixEvents, $evts => {
    let list = $evts;

    // chip filter
    if (activeChip !== 'all') {
      list = list.filter(e => {
        const t = eventType(e);
        if (activeChip === 'errors') return t === 'step_error' || t === 'overlap_detected';
        if (activeChip === 'governance') return t === 'governance';
        if (activeChip === 'artifacts') return t === 'artifact';
        if (activeChip === 'tasks') return t === 'task_claimed' || t === 'task_finished';
        if (activeChip === 'heartbeat') return t === 'heartbeat';
        return true;
      });
    }

    // alerts only
    if (alertsOnly) {
      list = list.filter(e => alertClass(e) !== '');
    }

    // text filter
    if (filterText.trim()) {
      const q = filterText.toLowerCase();
      list = list.filter(e =>
        (e.agent_id ?? '').toLowerCase().includes(q) ||
        (e.mission_id ?? '').toLowerCase().includes(q) ||
        eventType(e).toLowerCase().includes(q) ||
        summaryOf(e.payload).toLowerCase().includes(q)
      );
    }

    return list;
  });

  const errorCount = derived(matrixEvents, $evts =>
    $evts.filter(e => alertClass(e) === 'a-err').length
  );

  const govCount = derived(matrixEvents, $evts =>
    $evts.filter(e => alertClass(e) === 'a-gov').length
  );

  const warnCount = derived(matrixEvents, $evts =>
    $evts.filter(e => alertClass(e) === 'a-warn').length
  );

  // ── Helpers ──────────────────────────────────────────────────────────────────

  function eventType(e: { type?: string; event?: string }): string {
    return e.type ?? e.event ?? '';
  }

  function alertClass(e: { type?: string; event?: string }): string {
    const t = eventType(e);
    if (t === 'step_error') return 'a-err';
    if (t === 'overlap_detected') return 'a-warn';
    if (t === 'governance') return 'a-gov';
    return '';
  }

  function typeClass(e: { type?: string; event?: string }): string {
    const t = eventType(e);
    if (t === 'step_started') return 'ty-start';
    if (t === 'step_finished') return 'ty-finish';
    if (t === 'step_error') return 'ty-err';
    if (t === 'governance') return 'ty-gov';
    if (t === 'artifact') return 'ty-art';
    if (t === 'heartbeat') return 'ty-hb';
    if (t === 'task_claimed') return 'ty-claim';
    if (t === 'task_finished') return 'ty-done';
    if (t === 'overlap_detected') return 'ty-warn';
    return '';
  }

  function summaryOf(payload: unknown): string {
    if (!payload || typeof payload !== 'object') return String(payload ?? '');
    const p = payload as Record<string, unknown>;
    return String(p.summary ?? p.message ?? p.title ?? p.name ?? '').slice(0, 140)
      || JSON.stringify(payload).slice(0, 140);
  }

  function agentOf(e: { agent_id?: string; payload?: unknown }): string {
    if (e.agent_id) return e.agent_id;
    if (e.payload && typeof e.payload === 'object') {
      const p = e.payload as Record<string, unknown>;
      return String(p.agent ?? p.agent_id ?? '');
    }
    return '';
  }

  function contextOf(e: { mission_id?: string; payload?: unknown }): string {
    if (e.mission_id) return e.mission_id;
    if (e.payload && typeof e.payload === 'object') {
      const p = e.payload as Record<string, unknown>;
      return String(p.context ?? p.task_id ?? '');
    }
    return '';
  }

  function fmtTime(ts: number): string {
    return new Date(ts).toLocaleTimeString('en-US', {
      hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit'
    });
  }

  function fmtTimeFull(ts: number): string {
    const d = new Date(ts);
    return d.toLocaleTimeString('en-US', {
      hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit'
    }) + '.' + String(d.getMilliseconds()).padStart(3, '0');
  }

  // ── Selected event ───────────────────────────────────────────────────────────

  $effect(() => {
    // Reset selection when filter changes
    selectedIdx = null;
  });
</script>

<!-- Filter bar -->
<div id="filterbar">
  <div class="fi" class:focused={filterText.length > 0}>
    <span class="dim">/</span>
    <input
      type="text"
      placeholder="filter events..."
      bind:value={filterText}
    />
  </div>

  <span class="fsep">|</span>

  <button
    class="chip"
    class:on={activeChip === 'all'}
    onclick={() => (activeChip = 'all')}
  >All</button>

  <button
    class="chip"
    class:on-err={activeChip === 'errors'}
    onclick={() => (activeChip = activeChip === 'errors' ? 'all' : 'errors')}
  >Errors</button>

  <button
    class="chip"
    class:on-gov={activeChip === 'governance'}
    onclick={() => (activeChip = activeChip === 'governance' ? 'all' : 'governance')}
  >Governance</button>

  <button
    class="chip"
    onclick={() => (activeChip = activeChip === 'artifacts' ? 'all' : 'artifacts')}
    class:on={activeChip === 'artifacts'}
  >Artifacts</button>

  <button
    class="chip"
    onclick={() => (activeChip = activeChip === 'tasks' ? 'all' : 'tasks')}
    class:on={activeChip === 'tasks'}
  >Tasks</button>

  <button
    class="chip"
    onclick={() => (activeChip = activeChip === 'heartbeat' ? 'all' : 'heartbeat')}
    class:on={activeChip === 'heartbeat'}
  >Heartbeat</button>

  <span class="fsep">|</span>

  <button
    class="fi alerts-toggle"
    class:active-warn={alertsOnly}
    onclick={() => (alertsOnly = !alertsOnly)}
  >
    <span class="alert-dot" class:on={alertsOnly}></span>
    <span>Alerts only</span>
  </button>

  <div class="fr">
    <span>{$matrixEvents.length} events</span>
    <span class="dim">·</span>
    <span class="ok" style="font-size:11px">rate {$recentRate}/60</span>
    {#if $matrixStatus.connected}
      <span class="ok live">●</span>
      <span class="ok" style="font-weight:700;font-size:11px">LIVE</span>
    {:else}
      <span class="dim">○ offline</span>
    {/if}
  </div>
</div>

<!-- Content -->
<div id="content">

  <!-- Feed list -->
  <div id="feed-list">
    <div id="feed-hdr">
      <span>Time</span>
      <span>Agent</span>
      <span>Context</span>
      <span>Event</span>
      <span>Detail</span>
    </div>

    <div id="feed">
      {#if $filteredEvents.length === 0}
        <div class="feed-empty">
          <span class="dim">
            {filterText || activeChip !== 'all' ? 'No matching events.' : 'Waiting for events…'}
          </span>
        </div>
      {:else}
        {#each $filteredEvents as evt, i (evt.receivedAt + '-' + i)}
          {@const ac = alertClass(evt)}
          {@const tc = typeClass(evt)}
          {@const agent = agentOf(evt)}
          {@const ctx = contextOf(evt)}
          {@const summary = summaryOf(evt.payload)}
          {@const label = eventType(evt)}
          <div
            class="f-row {ac}"
            class:sel={selectedIdx === i}
            onclick={() => (selectedIdx = selectedIdx === i ? null : i)}
            role="row"
            tabindex="0"
            onkeydown={(e) => e.key === 'Enter' && (selectedIdx = selectedIdx === i ? null : i)}
          >
            <span class="f-time" class:err={ac === 'a-err'} class:warn={ac === 'a-warn'}>
              {fmtTime(evt.receivedAt)}
            </span>
            <span class="f-agent" class:err={ac === 'a-err'} class:warn={ac === 'a-warn'}>
              {agent || '—'}
            </span>
            <span class="f-ctx" class:err={ac === 'a-err'} class:warn={ac === 'a-warn'}>
              {ctx || '—'}
            </span>
            <span class="f-type {tc}">{label || 'event'}</span>
            <span class="f-msg" class:err={ac === 'a-err'} class:warn={ac === 'a-warn'}
                  class:ok={label === 'task_finished'} class:purple-txt={label === 'artifact'}>
              {summary}
            </span>
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <!-- Detail panel -->
  <div id="detail-panel">
    {#if selectedIdx !== null && $filteredEvents[selectedIdx]}
      {@const evt = $filteredEvents[selectedIdx]}
      {@const ac = alertClass(evt)}
      {@const label = eventType(evt)}
      {@const agent = agentOf(evt)}
      {@const ctx = contextOf(evt)}
      {@const payload = evt.payload}

      <div class="dp-hdr">
        {#if ac === 'a-err'}<span class="err">⚠</span>
        {:else if ac === 'a-gov'}<span class="purple">⬡</span>
        {:else if ac === 'a-warn'}<span class="warn">⚠</span>
        {:else}<span class="dim">○</span>{/if}
        <span class="t">{label || 'event'}</span>
        <span class="dim">·</span>
        <span>{fmtTime(evt.receivedAt)}</span>
      </div>

      <div class="dp-body">
        <div class="kv"><span class="kk">Time</span><span class="dim">{fmtTimeFull(evt.receivedAt)}</span></div>
        {#if agent}<div class="kv"><span class="kk">Agent</span><span>{agent}</span></div>{/if}
        {#if evt.domain_id}<div class="kv"><span class="kk">Domain</span><span class="muted">{evt.domain_id}</span></div>{/if}
        {#if evt.mission_id}<div class="kv"><span class="kk">Mission</span><span class="muted">{evt.mission_id}</span></div>{/if}
        {#if ctx && ctx !== evt.mission_id}<div class="kv"><span class="kk">Context</span><span class="muted">{ctx}</span></div>{/if}

        {#if payload && typeof payload === 'object' && Object.keys(payload).length > 0}
          <div class="d-sep"></div>
          <div class="d-sec">Payload</div>
          <div class="payload-block">
            {#each Object.entries(payload as Record<string, unknown>) as [k, v]}
              <div>
                <span class="pk">{k}</span>
                {' '}
                <span class:pv-err={k === 'error' || k === 'message' && ac === 'a-err'} class="pv">
                  {typeof v === 'string' ? v : JSON.stringify(v)}
                </span>
              </div>
            {/each}
          </div>
        {/if}

        {#if evt.status}
          <div class="d-sep"></div>
          <div class="d-sec">Status</div>
          <div class="kv"><span class="kk">Status</span><span>{evt.status}</span></div>
        {/if}

        {#if evt.agent_id}
          <div class="d-sep"></div>
          <div class="d-sec">Agent Context</div>
          <div class="ctx-log">
            <div><span class="muted">{fmtTime(evt.receivedAt)}</span> {label} — {summaryOf(payload)}</div>
          </div>
        {/if}
      </div>
    {:else}
      <div class="dp-empty">
        <span class="dim">Select a row to inspect</span>
      </div>
    {/if}
  </div>

</div>

<!-- Statusbar -->
<div id="statusbar">
  <span class="muted">{$matrixEvents.length} events</span>
  <span class="dim">·</span>
  {#if $errorCount > 0}
    <span class="err">{$errorCount} error{$errorCount === 1 ? '' : 's'}</span>
  {:else}
    <span class="dim">0 errors</span>
  {/if}
  {#if $govCount > 0}
    <span class="warn">{$govCount} governance</span>
  {/if}
  {#if $warnCount > 0}
    <span class="warn">{$warnCount} overlap</span>
  {/if}
  <div id="statusbar-right">
    <span>/ filter</span>
    <span class="dim">·</span>
    <span>click row to inspect</span>
    {#if $matrixStatus.connected}
      <span class="dim">·</span>
      <span class="ok live">●</span> <span class="ok">LIVE</span>
    {/if}
  </div>
</div>

<style>
  /* ── Filter bar ─────────────────────────────────────────────────────────── */

  #filterbar {
    height: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    flex-wrap: nowrap;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    padding: 0 12px;
    gap: 6px;
  }

  .fi {
    display: flex;
    align-items: center;
    gap: 5px;
    background: var(--base);
    border: 1px solid var(--border-mid);
    border-radius: 3px;
    padding: 2px 8px;
    font-size: 11px;
    color: var(--muted);
  }

  .fi.focused { border-color: var(--accent); }

  .fi input {
    background: transparent;
    border: none;
    outline: none;
    color: var(--text);
    font-family: inherit;
    font-size: 11px;
    width: 20ch;
    padding: 0;
  }

  .fi input::placeholder { color: var(--dim); }

  .fsep { color: var(--border); font-size: 14px; margin: 0 2px; }

  /* chip overrides for feed-specific states */
  .chip.on-err  { border-color: var(--err-border); color: var(--err); background: var(--err-bg); }
  .chip.on-gov  { border-color: var(--purple-border); color: var(--purple); background: var(--purple-bg); }

  .alerts-toggle {
    background: var(--base);
    border: 1px solid var(--border-mid);
    border-radius: 3px;
    padding: 2px 8px;
    font-size: 11px;
    color: var(--muted);
    display: flex;
    align-items: center;
    gap: 5px;
    cursor: pointer;
  }

  .alerts-toggle.active-warn {
    border-color: var(--warn-border);
    color: var(--warn);
    background: var(--warn-bg);
  }

  .alert-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--dim);
    display: inline-block;
    flex-shrink: 0;
  }

  .alert-dot.on { background: var(--warn); }

  .fr {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 11px;
    color: var(--dim);
    white-space: nowrap;
  }

  /* ── Feed list ──────────────────────────────────────────────────────────── */

  #feed-list {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 0;
  }

  #feed-hdr {
    display: grid;
    grid-template-columns: 68px 140px 160px 120px 1fr;
    gap: 0 6px;
    padding: 3px 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border-mid);
    color: var(--dim);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    flex-shrink: 0;
  }

  #feed {
    flex: 1;
    overflow-y: auto;
  }

  .feed-empty {
    padding: 24px 12px;
    font-size: 12px;
  }

  .f-row {
    display: grid;
    grid-template-columns: 68px 140px 160px 120px 1fr;
    gap: 0 6px;
    padding: 3px 12px;
    border-bottom: 1px solid var(--border);
    align-items: baseline;
    cursor: pointer;
  }

  .f-row:hover { background: var(--surface); }
  .f-row.sel { background: var(--surface-2); }

  .f-row.a-err  { border-left: 2px solid var(--err); background: #120a0a; }
  .f-row.a-warn { border-left: 2px solid var(--warn); background: #110f00; }
  .f-row.a-gov  { border-left: 2px solid var(--purple); background: #0a0f1a; }

  .f-row.sel.a-err  { background: #1f1010; }
  .f-row.sel.a-warn { background: #1f1a0a; }
  .f-row.sel.a-gov  { background: #141c30; }

  .f-time  { color: var(--dim); font-size: 11px; }
  .f-agent { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .f-ctx   { color: var(--dim); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .f-type  { font-size: 11px; font-weight: 600; white-space: nowrap; }
  .f-msg   { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .f-msg.purple-txt { color: var(--purple); }

  /* Event type colors */
  .ty-start  { color: var(--accent); }
  .ty-finish { color: var(--ok); }
  .ty-err    { color: var(--err); font-weight: 700; }
  .ty-gov    { color: var(--purple); font-weight: 700; }
  .ty-art    { color: var(--purple); }
  .ty-hb     { color: var(--border-mid); }
  .ty-claim  { color: var(--accent); }
  .ty-done   { color: var(--ok); font-weight: 700; }
  .ty-warn   { color: var(--warn); font-weight: 700; }

  /* ── Detail panel ───────────────────────────────────────────────────────── */

  #detail-panel {
    width: 380px;
    flex-shrink: 0;
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .dp-hdr {
    height: 28px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    padding: 0 12px;
    font-size: 11px;
    color: var(--muted);
  }

  .dp-hdr .t { color: var(--text); font-size: 12px; }

  .dp-body {
    flex: 1;
    overflow-y: auto;
    padding: 10px 12px;
  }

  .dp-empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
  }

  .d-sep { border-top: 1px solid var(--border); margin: 8px 0 6px; }

  .d-sec {
    font-size: 10px;
    color: var(--dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin-bottom: 6px;
  }

  .payload-block {
    background: var(--surface);
    border: 1px solid var(--border-mid);
    padding: 7px 10px;
    font-size: 11px;
    line-height: 1.7;
    font-family: inherit;
    margin-bottom: 6px;
    white-space: pre-wrap;
    word-break: break-all;
  }

  .pk { color: var(--accent); }
  .pv { color: var(--text); }
  .pv-err { color: var(--err); }

  .ctx-log { font-size: 11px; color: var(--dim); line-height: 1.9; }
</style>
