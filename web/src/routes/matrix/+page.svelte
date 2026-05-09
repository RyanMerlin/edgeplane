<script lang="ts">
  import { derived } from 'svelte/store';
  import { matrixEvents, matrixStatus } from '$lib/telemetry';

  const lastEvent = derived(matrixEvents, $evts => ($evts.length ? $evts[0] : null));
  const eventChunks = derived(matrixEvents, $evts =>
    $evts.map(event => ({
      label: event.type ?? 'matrix',
      status: event.status,
      detail: event.payload,
      time: new Date(event.receivedAt).toLocaleTimeString()
    }))
  );
</script>

<div class="glass-panel">
  <div class="grid">
    <div>
      <div class="status-chip">Matrix stream {$matrixStatus.connected ? 'live' : 'offline'}</div>
      <p class="muted">Rate limit: {$matrixStatus.rateLimit?.remaining ?? '—'} / {$matrixStatus.rateLimit?.limit ?? '—'}</p>
    </div>
    <div class="status-chip">Last event: {#if $lastEvent}{$lastEvent.time}{:else}waiting...{/if}</div>
  </div>
  <div class="matrix-timeline">
    {#each $eventChunks as chunk}
      <div class="event-pill">
        <small>{chunk.time}</small>
        <p>{chunk.label} - {chunk.status ?? 'pending'}</p>
        <p class="muted">{(chunk.detail as { summary?: string })?.summary ?? JSON.stringify(chunk.detail)}</p>
      </div>
    {/each}
  </div>
</div>
